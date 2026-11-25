//! End-to-end runs of the `flush` binary. Offline: nothing here logs in.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn flush(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_flush"))
        .args(args)
        .current_dir(dir)
        .env_remove("FLUXER_BOT_TOKEN")
        .output()
        .unwrap()
}

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flush-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

#[test]
fn help_version_and_usage_errors() {
    let dir = workdir("help");
    let help = flush(&dir, &[]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("flush init <directory>"));
    assert_eq!(
        stdout(&flush(&dir, &["--version"])).trim(),
        env!("CARGO_PKG_VERSION")
    );

    let unknown = flush(&dir, &["init", "bot", "--nope"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("[USAGE]"));
    assert!(!dir.join("bot").exists());

    let deferred = flush(&dir, &["functions", "--json"]);
    assert_eq!(deferred.status.code(), Some(1));
    assert_eq!(
        json(&String::from_utf8_lossy(&deferred.stderr))["code"],
        "UNSUPPORTED"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn templates_list_as_text_and_json() {
    let dir = workdir("templates");
    let listed = json(&stdout(&flush(&dir, &["templates", "--json"])));
    let names: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|template| template["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["simple", "moderation", "entertainment"]);
    assert!(listed[1]["commands"]
        .as_array()
        .unwrap()
        .contains(&"moderation/ban".into()));
    assert!(stdout(&flush(&dir, &["templates"])).contains("entertainment"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn init_check_doctor_and_run_guard() {
    let dir = workdir("init");
    let created = flush(&dir, &["init", "my-bot", "--template", "moderation"]);
    assert!(created.status.success(), "{created:?}");
    let project = dir.join("my-bot");
    assert!(project.join("src/commands/timeout.rs").is_file());
    assert_eq!(flush(&dir, &["init", "my-bot"]).status.code(), Some(1));

    let checked = flush(&dir, &["check", "my-bot", "--json"]);
    assert!(checked.status.success());
    let report = json(&stdout(&checked));
    assert_eq!(report["valid"], true);
    assert_eq!(report["config"]["name"], "my-bot");
    assert_eq!(report["config"]["prefix"], "!");

    let doctor = flush(&project, &["doctor", "--json"]);
    assert!(doctor.status.success(), "{doctor:?}");
    let report = json(&stdout(&doctor));
    assert_eq!(report["offline"], true);
    let token = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "token")
        .unwrap();
    assert_eq!(token["status"], "warn");

    // Without a token `run` stops before building or connecting.
    let run = flush(&project, &["run"]);
    assert_eq!(run.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&run.stderr).contains("[TOKEN_MISSING]"));

    // A failing build keeps stderr a single JSON object.
    std::fs::write(project.join("Cargo.toml"), "this is not a manifest").unwrap();
    let broken = flush(&project, &["check", "--json", "--build"]);
    assert_eq!(broken.status.code(), Some(1));
    assert_eq!(
        json(&String::from_utf8_lossy(&broken.stderr))["code"],
        "BUILD_FAILED"
    );

    std::fs::write(
        project.join("flush.toml"),
        "name = \"my-bot\"\nunknown = true\n",
    )
    .unwrap();
    let invalid = flush(&project, &["check", "--json"]);
    assert_eq!(invalid.status.code(), Some(1));
    let report = json(&String::from_utf8_lossy(&invalid.stderr));
    assert_eq!(report["valid"], false);
    assert_eq!(report["code"], "CONFIG_INVALID");
    assert_eq!(flush(&project, &["doctor"]).status.code(), Some(1));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_without_config_reports_config_missing() {
    let dir = workdir("missing");
    let missing = flush(&dir, &["check", "--json"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        json(&String::from_utf8_lossy(&missing.stderr))["code"],
        "CONFIG_MISSING"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Generates every scaffold against the local checkout and compiles, lints
/// and tests it. Slow, and needs the crates.io cache, so it is opt-in:
/// `cargo test -p flush-cli --test cli -- --ignored`.
#[test]
#[ignore = "builds every generated project"]
fn generated_projects_compile_and_pass_their_tests() {
    let dir = workdir("build");
    let target = dir.join("target");
    let templates: [&[&str]; 4] = [
        &[],
        &["--template", "simple"],
        &["--template", "moderation"],
        &["--template", "entertainment"],
    ];
    for (index, template) in templates.iter().enumerate() {
        let name = format!("bot{index}");
        let mut args = vec!["init", name.as_str(), "--local"];
        args.extend_from_slice(template);
        assert!(flush(&dir, &args).status.success());
        let project = dir.join(&name);
        let cargo = |args: &[&str]| {
            let status = Command::new("cargo")
                .args(args)
                .current_dir(&project)
                .env("CARGO_TARGET_DIR", &target)
                .status()
                .unwrap();
            assert!(status.success(), "cargo {args:?} failed for {template:?}");
        };
        cargo(&["fmt", "--check"]);
        cargo(&["clippy", "--all-targets", "--", "-D", "warnings"]);
        cargo(&["test"]);
        let checked = Command::new(env!("CARGO_BIN_EXE_flush"))
            .args(["check", "--build"])
            .current_dir(&project)
            .env("CARGO_TARGET_DIR", &target)
            .status()
            .unwrap();
        assert!(checked.success());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
