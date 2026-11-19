//! flush-cli: the `flush` command line (mirrors Yuzuha's CLI).
//!
//! - `init` / `templates`: project scaffolding ([`scaffold`], [`templates`])
//! - `check` / `doctor`: offline validation of `flush.toml` ([`doctor`])
//! - `data`: JSON store maintenance ([`data`])
//! - `run` / `dev`: build, run and restart-on-change lifecycle ([`runner`])

pub mod arguments;
pub mod data;
pub mod doctor;
pub mod error;
pub mod runner;
pub mod scaffold;
pub mod templates;

use std::path::Path;

use flush_framework::{find_config, ProjectConfig};
use serde_json::json;

pub use arguments::Action;
pub use error::CliError;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn pretty(value: &impl serde::Serialize) -> Result<String, CliError> {
    Ok(serde_json::to_string_pretty(value)?)
}

fn check(input: Option<&Path>, json: bool, build: bool) -> Result<i32, CliError> {
    let path = find_config(input)?;
    let config = ProjectConfig::load(&path)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    if build {
        let status =
            std::process::Command::new(std::env::var_os("CARGO").unwrap_or("cargo".into()))
                .args(["check", "--quiet"])
                .current_dir(dir)
                .stdout(std::process::Stdio::null())
                // With --json, stderr carries exactly one JSON object on failure.
                .stderr(if json {
                    std::process::Stdio::null()
                } else {
                    std::process::Stdio::inherit()
                })
                .status()
                .map_err(|e| CliError::new("BUILD_FAILED", format!("Cannot run cargo: {e}.")))?;
        if !status.success() {
            return Err(CliError::new(
                "BUILD_FAILED",
                if json {
                    "cargo check failed. Run it without --json to see the compiler diagnostics."
                } else {
                    "cargo check failed."
                },
            ));
        }
    }
    if json {
        println!(
            "{}",
            pretty(
                &json!({ "valid": true, "version": VERSION, "built": build, "config": config })
            )?
        );
    } else {
        println!(
            "{} is valid (prefix {}){}. No login performed.",
            config.name,
            config.prefix,
            if build {
                " and the project compiles"
            } else {
                ""
            }
        );
    }
    Ok(0)
}

/// Runs an already parsed action and returns the process exit code.
pub fn execute(action: &Action) -> Result<i32, CliError> {
    match action {
        Action::Help => println!("{}", arguments::USAGE),
        Action::Version => println!("{VERSION}"),
        Action::Templates { json } => {
            let catalog = templates::catalog();
            if *json {
                println!("{}", pretty(&catalog)?);
            } else {
                for template in catalog {
                    println!(
                        "{:<16} {} ({} command files)",
                        template.name,
                        template.description,
                        template.commands.len()
                    );
                }
            }
        }
        Action::Init {
            directory,
            template,
            local,
        } => {
            let root = scaffold::scaffold(directory, template.as_deref(), *local)?;
            println!(
                "Created: {}\nEnter the directory and run flush check --build and cargo test.",
                root.display()
            );
        }
        Action::Check { input, json, build } => return check(input.as_deref(), *json, *build),
        Action::Doctor { input, json } => {
            let report = doctor::doctor(input.as_deref());
            if *json {
                println!("{}", pretty(&report)?);
            } else {
                for check in &report.checks {
                    println!("{} {}: {}", check.status.label(), check.name, check.message);
                }
            }
            return Ok(if report.ok { 0 } else { 1 });
        }
        Action::Data {
            operation,
            input,
            file,
            apply,
            policy,
        } => {
            let report = data::data_command(*operation, input, file.as_deref(), *apply, *policy)?;
            println!("{}", pretty(&report)?);
        }
        Action::Run { input } => return runner::run(input.as_deref()),
        Action::Dev { input } => return runner::dev(input.as_deref()),
    }
    Ok(0)
}

/// CLI entry point: parses `args` (without the program name), runs the action
/// and reports failures on stderr, as JSON when the action asked for JSON.
pub fn main(args: &[String]) -> i32 {
    let json = args.iter().any(|arg| arg == "--json");
    let (result, json) = match arguments::parse(args) {
        Ok(action) => {
            let json = action.json();
            (execute(&action), json)
        }
        Err(error) => (Err(error), json),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            if json {
                let report =
                    json!({ "valid": false, "code": error.code, "message": error.message });
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_default()
                );
            } else {
                eprintln!("error {error}");
            }
            1
        }
    }
}
