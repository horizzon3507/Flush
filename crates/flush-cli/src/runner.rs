//! `flush run` and `flush dev`: build the project, run the bot binary with
//! the project's `.env`, and (in dev) rebuild and restart when sources change.
//!
//! A bot is a compiled crate, so "reload" means rebuild and restart. A build
//! that fails is refused and the running version stays up.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use flush_framework::{find_config, read_env_file, ProjectConfig, CONFIG_ENV};

use crate::error::CliError;

/// How long a stopping bot gets before it is killed.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);
const DEBOUNCE: Duration = Duration::from_millis(150);

/// A project ready to run: validated config, directory and child environment.
#[derive(Debug)]
pub struct Project {
    pub dir: PathBuf,
    pub config_path: PathBuf,
    pub config: ProjectConfig,
    /// `.env` entries not already set in the process environment.
    pub env: Vec<(String, String)>,
}

/// Loads and validates everything `run`/`dev` need before building.
pub fn prepare(input: Option<&Path>) -> Result<Project, CliError> {
    let config_path = find_config(input)?;
    let dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let config = ProjectConfig::load(&config_path)?;
    if !dir.join("Cargo.toml").is_file() {
        return Err(CliError::new(
            "PROJECT_MISSING",
            "No Cargo.toml next to the config; run and dev need a Cargo project.",
        ));
    }
    let env: Vec<(String, String)> = read_env_file(&dir)?
        .into_iter()
        .filter(|(key, _)| std::env::var_os(key).is_none())
        .collect();
    let token = std::env::var("FLUXER_BOT_TOKEN").ok().or_else(|| {
        env.iter()
            .find(|(key, _)| key == "FLUXER_BOT_TOKEN")
            .map(|(_, value)| value.clone())
    });
    if token.is_none_or(|token| token.trim().is_empty()) {
        return Err(CliError::new(
            "TOKEN_MISSING",
            "FLUXER_BOT_TOKEN is not set. Copy .env.example to .env and fill it in.",
        ));
    }
    Ok(Project {
        dir,
        config_path,
        config,
        env,
    })
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// Name and path of every binary in cargo's `--message-format=json` output.
pub fn binaries_from_messages(output: &str) -> Vec<(String, PathBuf)> {
    let mut binaries: Vec<(String, PathBuf)> = Vec::new();
    let artifacts = output
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| {
            message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        });
    for message in artifacts {
        let (Some(name), Some(path)) = (
            message["target"]["name"].as_str(),
            message["executable"].as_str(),
        ) else {
            continue;
        };
        binaries.retain(|(known, _)| known != name);
        binaries.push((name.to_owned(), PathBuf::from(path)));
    }
    binaries
}

/// `default-run` of the project's manifest. Like `cargo run`, nothing else
/// picks between several binaries, not even one named after the package.
pub fn default_binary(manifest: &str) -> Option<String> {
    let manifest: toml::Table = manifest.parse().ok()?;
    manifest
        .get("package")?
        .get("default-run")?
        .as_str()
        .map(str::to_owned)
}

/// The bot binary: the only one built, or the one `cargo run` would pick.
pub fn select_binary(
    mut binaries: Vec<(String, PathBuf)>,
    default: Option<&str>,
) -> Result<PathBuf, CliError> {
    if binaries.len() == 1 {
        return Ok(binaries.remove(0).1);
    }
    if binaries.is_empty() {
        return Err(CliError::new(
            "BUILD_FAILED",
            "The project builds no binary.",
        ));
    }
    binaries
        .into_iter()
        .find(|(name, _)| Some(name.as_str()) == default)
        .map(|(_, path)| path)
        .ok_or_else(|| {
            CliError::new(
                "BUILD_FAILED",
                "The project builds several binaries; set `default-run` in Cargo.toml to the bot.",
            )
        })
}

/// Builds the project and returns the bot binary. Compiler output goes to
/// stderr. When `stopping` is raised the build is killed and fails with
/// `INTERRUPTED`.
pub fn build(dir: &Path, release: bool, stopping: &AtomicBool) -> Result<PathBuf, CliError> {
    build_with(cargo(), dir, release, stopping)
}

fn build_with(
    mut command: Command,
    dir: &Path,
    release: bool,
    stopping: &AtomicBool,
) -> Result<PathBuf, CliError> {
    command
        .arg("build")
        .arg("--message-format=json-render-diagnostics")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if release {
        command.arg("--release");
    }
    // Its own process group, so an interrupt reaches rustc, linkers and
    // build scripts too, and a terminal Ctrl+C is ours to handle.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut cargo = command
        .spawn()
        .map_err(|e| CliError::new("BUILD_FAILED", format!("Cannot run cargo: {e}.")))?;
    let mut stdout = cargo.stdout.take();
    let messages = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = std::io::Read::read_to_string(stdout, &mut text);
        }
        text
    });
    let status = loop {
        if let Some(status) = cargo.try_wait()? {
            break status;
        }
        if stopping.load(Ordering::Relaxed) {
            kill_build(&mut cargo);
            return Err(interruption());
        }
        std::thread::sleep(POLL);
    };
    let messages = messages.join().unwrap_or_default();
    if !status.success() {
        // Where the signal also reached cargo, it may have died before the
        // flag was seen: that is still an interrupt, not a failed build.
        if stopping.load(Ordering::Relaxed) {
            kill_build(&mut cargo);
            return Err(interruption());
        }
        return Err(CliError::new("BUILD_FAILED", "cargo build failed."));
    }
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
    select_binary(
        binaries_from_messages(&messages),
        default_binary(&manifest).as_deref(),
    )
}

fn interruption() -> CliError {
    CliError::new("INTERRUPTED", "The build was interrupted.")
}

/// Kills cargo and, on Unix, everything it started (its process group).
fn kill_build(cargo: &mut Child) {
    #[cfg(unix)]
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", cargo.id())])
        .stderr(Stdio::null())
        .status();
    let _ = cargo.kill();
    let _ = cargo.wait();
}

/// A signal during a build ends the command cleanly; other failures stay errors.
fn interrupted(error: CliError) -> Result<i32, CliError> {
    if error.code == "INTERRUPTED" {
        Ok(0)
    } else {
        Err(error)
    }
}

/// The bot process: project directory as working directory, `.env` values
/// and the path of the validated config in its environment.
pub fn bot_command(executable: &Path, project: &Project) -> Command {
    let mut command = Command::new(executable);
    command
        .current_dir(&project.dir)
        .envs(project.env.iter().map(|(key, value)| (key, value)))
        .env(CONFIG_ENV, &project.config_path);
    command
}

fn spawn(executable: &Path, project: &Project) -> Result<Child, CliError> {
    bot_command(executable, project)
        .spawn()
        .map_err(|e| CliError::new("RUN_FAILED", format!("Cannot start the bot: {e}.")))
}

/// Asks the bot to stop (SIGTERM on Unix), waits up to `timeout`, then kills it.
pub fn stop(child: &mut Child, timeout: Duration) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(POLL);
        }
    }
    #[cfg(not(unix))]
    let _ = timeout;
    let _ = child.kill();
    let _ = child.wait();
}

/// Modification state of the files that should trigger a restart.
pub type Snapshot = BTreeMap<PathBuf, (SystemTime, u64)>;

fn visit(path: &Path, out: &mut Snapshot) {
    let Ok(info) = std::fs::metadata(path) else {
        return;
    };
    if info.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            let rust = child.extension().is_some_and(|e| e == "rs");
            if child.is_dir() || rust {
                visit(&child, out);
            }
        }
    } else {
        let modified = info.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        out.insert(path.to_path_buf(), (modified, info.len()));
    }
}

/// Watches `src/**/*.rs`, `Cargo.toml`, the config file and `.env`.
pub fn snapshot(dir: &Path, config_path: &Path) -> Snapshot {
    let mut out = Snapshot::new();
    visit(&dir.join("src"), &mut out);
    visit(&dir.join("Cargo.toml"), &mut out);
    visit(&dir.join(".env"), &mut out);
    visit(config_path, &mut out);
    out
}

fn signals() -> Result<Arc<AtomicBool>, CliError> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, flag.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, flag.clone())?;
    Ok(flag)
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

/// Production run: release build, then the bot until it exits or a signal arrives.
pub fn run(input: Option<&Path>) -> Result<i32, CliError> {
    let project = prepare(input)?;
    let stopping = signals()?;
    let executable = match build(&project.dir, true, &stopping) {
        Ok(executable) => executable,
        Err(error) => return interrupted(error),
    };
    let mut child = spawn(&executable, &project)?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(exit_code(status));
        }
        if stopping.load(Ordering::Relaxed) {
            stop(&mut child, SHUTDOWN_TIMEOUT);
            return Ok(0);
        }
        std::thread::sleep(POLL);
    }
}

/// Development run: debug build, restart on source, config or `.env` changes.
pub fn dev(input: Option<&Path>) -> Result<i32, CliError> {
    let config_path = find_config(input)?;
    let dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    // Taken before `.env` is read and the build starts, so an edit made
    // while the first build runs still triggers a reload.
    let mut seen = snapshot(&dir, &config_path);
    let mut project = prepare(Some(&config_path))?;
    let stopping = signals()?;
    let executable = match build(&project.dir, false, &stopping) {
        Ok(executable) => executable,
        Err(error) => return interrupted(error),
    };
    let mut child = Some(spawn(&executable, &project)?);
    eprintln!("flush dev: watching {} for changes.", project.dir.display());
    loop {
        if stopping.load(Ordering::Relaxed) {
            if let Some(child) = child.as_mut() {
                stop(child, SHUTDOWN_TIMEOUT);
            }
            return Ok(0);
        }
        if let Some(running) = child.as_mut() {
            if let Some(status) = running.try_wait()? {
                eprintln!("flush dev: the bot exited ({status}); waiting for changes.");
                child = None;
            }
        }
        let mut current = snapshot(&project.dir, &project.config_path);
        if current != seen {
            // Editors write in bursts: wait until the tree stops changing.
            loop {
                std::thread::sleep(DEBOUNCE);
                let next = snapshot(&project.dir, &project.config_path);
                if next == current {
                    break;
                }
                current = next;
            }
            seen = current;
            let rebuilt = prepare(Some(&project.config_path)).and_then(|next| {
                build(&next.dir, false, &stopping).map(|executable| (next, executable))
            });
            match rebuilt {
                Ok(_) if stopping.load(Ordering::Relaxed) => {}
                Ok((next, executable)) => {
                    if let Some(mut old) = child.take() {
                        stop(&mut old, SHUTDOWN_TIMEOUT);
                    }
                    project = next;
                    child = Some(spawn(&executable, &project)?);
                    eprintln!("flush dev: restarted.");
                }
                // The signal is handled at the top of the loop, which stops the bot.
                Err(error) if error.code == "INTERRUPTED" => {}
                Err(error) => {
                    eprintln!("flush dev: reload refused: {error} The previous version stays up.")
                }
            }
        }
        std::thread::sleep(POLL * 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flush-runner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/commands")).unwrap();
        dir
    }

    #[test]
    fn picks_the_binary_cargo_run_would_start() {
        let artifact = |kind: &str, name: &str, path: &str| {
            format!(
                "{{\"reason\":\"compiler-artifact\",\"target\":{{\"kind\":[\"{kind}\"],\"name\":\"{name}\"}},\"executable\":{path}}}\n"
            )
        };
        let single = format!(
            "{}not json\n{}{{\"reason\":\"build-finished\",\"success\":true}}\n",
            artifact("lib", "dep", "null"),
            artifact("bin", "bot", "\"/t/debug/bot\""),
        );
        let binaries = binaries_from_messages(&single);
        assert_eq!(
            binaries,
            vec![("bot".to_owned(), PathBuf::from("/t/debug/bot"))]
        );
        assert_eq!(
            select_binary(binaries, None).unwrap(),
            PathBuf::from("/t/debug/bot")
        );

        // Cargo reports binaries in build order, so the helper may come last.
        let several = format!(
            "{}{}",
            artifact("bin", "bot", "\"/t/debug/bot\""),
            artifact("bin", "migrate", "\"/t/debug/migrate\""),
        );
        let pick = |manifest: &str| {
            select_binary(
                binaries_from_messages(&several),
                default_binary(manifest).as_deref(),
            )
        };
        assert_eq!(
            pick("[package]\nname = \"bot\"\n").unwrap_err().code,
            "BUILD_FAILED"
        );
        assert_eq!(
            pick("[package]\nname = \"bot\"\ndefault-run = \"bot\"\n").unwrap(),
            PathBuf::from("/t/debug/bot")
        );
        assert_eq!(
            pick("[package]\nname = \"tools\"\ndefault-run = \"migrate\"\n").unwrap(),
            PathBuf::from("/t/debug/migrate")
        );
        assert_eq!(
            pick("[package]\nname = \"tools\"\n").unwrap_err().code,
            "BUILD_FAILED"
        );
        assert_eq!(
            select_binary(Vec::new(), None).unwrap_err().code,
            "BUILD_FAILED"
        );
    }

    #[test]
    fn snapshot_tracks_sources_config_and_env() {
        let dir = temp("snapshot");
        let config = dir.join("flush.toml");
        std::fs::write(&config, "name = \"demo\"\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("src/notes.txt"), "ignored").unwrap();
        let before = snapshot(&dir, &config);
        assert_eq!(before.len(), 2);
        assert_eq!(before, snapshot(&dir, &config));

        std::fs::write(dir.join("src/commands/ping.rs"), "pub struct Ping;\n").unwrap();
        let added = snapshot(&dir, &config);
        assert_eq!(added.len(), 3);
        std::fs::write(dir.join(".env"), "FLUXER_BOT_TOKEN=x\n").unwrap();
        std::fs::write(&config, "name = \"renamed\"\n").unwrap();
        let changed = snapshot(&dir, &config);
        assert_eq!(changed.len(), 4);
        assert_ne!(changed[&config], added[&config]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prepare_validates_before_building() {
        let dir = temp("prepare");
        assert_eq!(prepare(Some(&dir)).unwrap_err().code, "CONFIG_MISSING");
        std::fs::write(dir.join("flush.toml"), "name = \"demo\"\nprefix = \"\"\n").unwrap();
        assert_eq!(prepare(Some(&dir)).unwrap_err().code, "CONFIG_INVALID");
        std::fs::write(dir.join("flush.toml"), "name = \"demo\"\n").unwrap();
        assert_eq!(prepare(Some(&dir)).unwrap_err().code, "PROJECT_MISSING");
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        if std::env::var_os("FLUXER_BOT_TOKEN").is_none() {
            assert_eq!(prepare(Some(&dir)).unwrap_err().code, "TOKEN_MISSING");
            std::fs::write(dir.join(".env"), "FLUXER_BOT_TOKEN=abc\nPREFIX=?\n").unwrap();
            let project = prepare(Some(&dir)).unwrap();
            assert!(project.env.iter().any(|(key, _)| key == "FLUXER_BOT_TOKEN"));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_signal_interrupts_the_build() {
        let dir = temp("interrupt");
        let fake = dir.join("cargo");
        std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let stopping = AtomicBool::new(true);
        let started = Instant::now();
        let error = build_with(Command::new(&fake), &dir, false, &stopping).unwrap_err();
        assert_eq!(error.code, "INTERRUPTED");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(interrupted(error), Ok(0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_interrupt_stops_what_cargo_started() {
        let dir = temp("descendants");
        let fake = dir.join("cargo");
        // Stands in for rustc: a child that outlives a killed parent, and
        // shows it is alive by writing to a file. It gives up by itself after
        // 100 beats, so a failed test leaves nothing running.
        let script =
            "#!/bin/sh\n(for beat in $(seq 100); do echo beat >> beats; sleep 0.1; done) &\nwait\n";
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let stopping = Arc::new(AtomicBool::new(false));
        let raise = stopping.clone();
        let beats = dir.join("beats");
        let written = move || std::fs::metadata(&beats).map_or(0, |beats| beats.len());
        let started = written.clone();
        let raiser = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while started() == 0 && Instant::now() < deadline {
                std::thread::sleep(POLL);
            }
            raise.store(true, Ordering::Relaxed);
        });
        let error = build_with(Command::new(&fake), &dir, false, &stopping).unwrap_err();
        raiser.join().unwrap();
        assert_eq!(error.code, "INTERRUPTED");
        let at_stop = written();
        assert!(at_stop > 0);
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(written(), at_stop);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_build_that_died_from_the_signal_is_an_interrupt() {
        let dir = temp("died");
        let fake = dir.join("cargo");
        std::fs::write(&fake, "#!/bin/sh\nexit 130\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let code = |stopping: bool| {
            // Let the fake exit before the flag is looked at.
            let flag = Arc::new(AtomicBool::new(false));
            let raise = flag.clone();
            let raiser = std::thread::spawn(move || raise.store(stopping, Ordering::Relaxed));
            raiser.join().unwrap();
            build_with(Command::new(&fake), &dir, false, &flag)
                .unwrap_err()
                .code
        };
        assert_eq!(code(false), "BUILD_FAILED");
        assert_eq!(code(true), "INTERRUPTED");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bot_command_names_the_validated_config() {
        let project = Project {
            dir: PathBuf::from("/srv/bot"),
            config_path: PathBuf::from("/srv/bot/config/bot.toml"),
            config: ProjectConfig::parse("name = \"demo\"\n").unwrap(),
            env: vec![("GREETING".into(), "hi".into())],
        };
        let command = bot_command(Path::new("/t/bot"), &project);
        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.contains(&(
            CONFIG_ENV.as_ref(),
            Some("/srv/bot/config/bot.toml".as_ref())
        )));
        assert!(envs.contains(&("GREETING".as_ref(), Some("hi".as_ref()))));
        assert_eq!(command.get_current_dir(), Some(Path::new("/srv/bot")));
    }

    #[cfg(unix)]
    #[test]
    fn stop_terminates_a_running_child() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let started = Instant::now();
        stop(&mut child, Duration::from_secs(5));
        assert!(child.try_wait().unwrap().is_some());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
