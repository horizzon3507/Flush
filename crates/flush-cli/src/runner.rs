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

/// Path of the binary named in cargo's `--message-format=json` output.
pub fn executable_from_messages(output: &str) -> Option<PathBuf> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| {
            message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .filter_map(|message| message["executable"].as_str().map(PathBuf::from))
        .next_back()
}

/// Builds the project and returns the bot binary. Compiler output goes to stderr.
pub fn build(dir: &Path, release: bool) -> Result<PathBuf, CliError> {
    let mut command = cargo();
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
    let output = command
        .output()
        .map_err(|e| CliError::new("BUILD_FAILED", format!("Cannot run cargo: {e}.")))?;
    if !output.status.success() {
        return Err(CliError::new("BUILD_FAILED", "cargo build failed."));
    }
    executable_from_messages(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| CliError::new("BUILD_FAILED", "The project builds no binary."))
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
    let executable = build(&project.dir, true)?;
    let stopping = signals()?;
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
    let mut project = prepare(input)?;
    let executable = build(&project.dir, false)?;
    let stopping = signals()?;
    let mut child = Some(spawn(&executable, &project)?);
    let mut seen = snapshot(&project.dir, &project.config_path);
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
            let rebuilt = prepare(Some(&project.config_path))
                .and_then(|next| build(&next.dir, false).map(|executable| (next, executable)));
            match rebuilt {
                Ok((next, executable)) => {
                    if let Some(mut old) = child.take() {
                        stop(&mut old, SHUTDOWN_TIMEOUT);
                    }
                    project = next;
                    child = Some(spawn(&executable, &project)?);
                    eprintln!("flush dev: restarted.");
                }
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
    fn picks_the_binary_from_cargo_messages() {
        let output = concat!(
            "{\"reason\":\"compiler-artifact\",\"target\":{\"kind\":[\"lib\"]},\"executable\":null}\n",
            "not json\n",
            "{\"reason\":\"compiler-artifact\",\"target\":{\"kind\":[\"bin\"]},\"executable\":\"/t/debug/bot\"}\n",
            "{\"reason\":\"build-finished\",\"success\":true}\n",
        );
        assert_eq!(
            executable_from_messages(output),
            Some(PathBuf::from("/t/debug/bot"))
        );
        assert_eq!(
            executable_from_messages("{\"reason\":\"build-finished\"}"),
            None
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
