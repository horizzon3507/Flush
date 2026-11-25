//! Argument parsing. Unknown or repeated options and extra arguments are
//! errors, as in Yuzuha's CLI.

use std::path::PathBuf;

use crate::error::CliError;

pub const USAGE: &str = "\
Flush: command line for Fluxer bots

Usage:
  flush help | --help                          Show this help
  flush --version                              Show the version
  flush init <directory> [--template <name>] [--local]
                                               Create a project in a new directory
  flush templates [--json]                     List the bot templates
  flush check [config or directory] [--json] [--build]
                                               Validate the config without logging in
  flush doctor [config or directory] [--json]  Offline diagnosis, never prints the token
  flush data <stats|backup|restore> <store.json> [--file <path>] [--apply] [--policy <replace|merge>]
                                               Inspect, back up and restore a JSON store
  flush run [config or directory]              Load .env and the config, build and run the bot
  flush dev [config or directory]              Run and restart when the sources change";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataOperation {
    Stats,
    Backup,
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestorePolicy {
    Replace,
    Merge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Help,
    Version,
    Init {
        directory: PathBuf,
        template: Option<String>,
        local: bool,
    },
    Templates {
        json: bool,
    },
    Check {
        input: Option<PathBuf>,
        json: bool,
        build: bool,
    },
    Doctor {
        input: Option<PathBuf>,
        json: bool,
    },
    Data {
        operation: DataOperation,
        input: PathBuf,
        file: Option<PathBuf>,
        apply: bool,
        policy: RestorePolicy,
    },
    Run {
        input: Option<PathBuf>,
    },
    Dev {
        input: Option<PathBuf>,
    },
}

impl Action {
    /// Whether failures should be reported as JSON on stderr.
    pub fn json(&self) -> bool {
        match self {
            Self::Templates { json } | Self::Check { json, .. } | Self::Doctor { json, .. } => {
                *json
            }
            Self::Data { .. } => true,
            _ => false,
        }
    }
}

fn usage(message: &str) -> CliError {
    CliError::new("USAGE", format!("{message}\n\n{USAGE}"))
}

fn unsupported(what: &str, item: &str) -> CliError {
    CliError::new(
        "UNSUPPORTED",
        format!("{what} is not available in Flush yet: it depends on {item} (see docs/PARITY.md)."),
    )
}

fn deferred_option(option: &str) -> Option<CliError> {
    match option {
        "--js" | "--ts" => Some(CliError::new(
            "UNSUPPORTED",
            format!("{option} does not apply: Flush projects are Rust crates."),
        )),
        "--functions" => Some(unsupported(
            "--functions",
            "the function-mode scripting language",
        )),
        "--shards" | "--shards-per-process" => {
            Some(unsupported(option, "the sharding and cluster runtime"))
        }
        "--health-port" | "--health-host" => Some(unsupported(option, "the health server")),
        _ => None,
    }
}

fn bad_option(option: &str) -> CliError {
    deferred_option(option)
        .unwrap_or_else(|| usage(&format!("Unknown or repeated option: {option}.")))
}

struct Flags<'a> {
    rest: std::slice::Iter<'a, String>,
}

impl<'a> Flags<'a> {
    fn value(&mut self, option: &str) -> Result<&'a str, CliError> {
        match self.rest.next() {
            Some(value) if !value.starts_with('-') && !value.is_empty() => Ok(value),
            _ => Err(usage(&format!("{option} needs a value."))),
        }
    }
}

fn set_flag(slot: &mut bool, option: &str) -> Result<(), CliError> {
    if *slot {
        return Err(bad_option(option));
    }
    *slot = true;
    Ok(())
}

fn set_input(slot: &mut Option<PathBuf>, value: &str) -> Result<(), CliError> {
    if value.starts_with('-') {
        return Err(bad_option(value));
    }
    if slot.is_some() {
        return Err(usage(&format!("Unexpected extra argument: {value}.")));
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

pub fn parse(args: &[String]) -> Result<Action, CliError> {
    let Some((action, rest)) = args.split_first() else {
        return Ok(Action::Help);
    };
    let mut flags = Flags { rest: rest.iter() };
    match action.as_str() {
        "help" | "--help" | "-h" | "--version" | "-V" => {
            if let Some(extra) = rest.first() {
                return Err(usage(&format!("Unexpected extra argument: {extra}.")));
            }
            Ok(if action.contains('h') {
                Action::Help
            } else {
                Action::Version
            })
        }
        "init" => {
            let (mut directory, mut template, mut local) = (None, None, false);
            while let Some(value) = flags.rest.next() {
                match value.as_str() {
                    "--local" => set_flag(&mut local, value)?,
                    "--template" if template.is_none() => {
                        template = Some(flags.value("--template")?.to_owned());
                    }
                    other => set_input(&mut directory, other)?,
                }
            }
            let directory = directory.ok_or_else(|| usage("Use init <directory>."))?;
            Ok(Action::Init {
                directory,
                template,
                local,
            })
        }
        "templates" => {
            let mut json = false;
            for value in flags.rest {
                match value.as_str() {
                    "--json" => set_flag(&mut json, value)?,
                    other if other.starts_with('-') => return Err(bad_option(other)),
                    other => return Err(usage(&format!("Unexpected extra argument: {other}."))),
                }
            }
            Ok(Action::Templates { json })
        }
        "check" | "doctor" => {
            let (mut input, mut json, mut build) = (None, false, false);
            for value in flags.rest {
                match value.as_str() {
                    "--json" => set_flag(&mut json, value)?,
                    "--build" if action == "check" => set_flag(&mut build, value)?,
                    other => set_input(&mut input, other)?,
                }
            }
            Ok(if action == "check" {
                Action::Check { input, json, build }
            } else {
                Action::Doctor { input, json }
            })
        }
        "data" => {
            let operation = match flags.rest.next().map(String::as_str) {
                Some("stats") => DataOperation::Stats,
                Some("backup") => DataOperation::Backup,
                Some("restore") => DataOperation::Restore,
                Some("cleanup") => {
                    return Err(unsupported(
                        "data cleanup",
                        "the variable service on the SQLite store (the JSON stores hold nothing that expires)",
                    ))
                }
                _ => return Err(usage("Use data <stats|backup|restore> <store.json>.")),
            };
            let input = match flags.rest.next() {
                Some(value) if !value.starts_with('-') => PathBuf::from(value),
                _ => return Err(usage("Use data <stats|backup|restore> <store.json>.")),
            };
            let (mut file, mut apply, mut policy, mut json) = (None, false, None, false);
            while let Some(value) = flags.rest.next() {
                match value.as_str() {
                    "--json" => set_flag(&mut json, value)?,
                    "--apply" if operation == DataOperation::Restore => {
                        set_flag(&mut apply, value)?
                    }
                    "--file" if operation != DataOperation::Stats && file.is_none() => {
                        file = Some(PathBuf::from(flags.value("--file")?));
                    }
                    "--policy" if operation == DataOperation::Restore && policy.is_none() => {
                        policy = Some(match flags.value("--policy")? {
                            "replace" => RestorePolicy::Replace,
                            "merge" => RestorePolicy::Merge,
                            _ => return Err(usage("Policy must be replace or merge.")),
                        });
                    }
                    other if other.starts_with('-') => return Err(bad_option(other)),
                    other => return Err(usage(&format!("Unexpected extra argument: {other}."))),
                }
            }
            if operation == DataOperation::Restore && file.is_none() {
                return Err(usage("Restore needs --file <backup.json>."));
            }
            Ok(Action::Data {
                operation,
                input,
                file,
                apply,
                policy: policy.unwrap_or(RestorePolicy::Replace),
            })
        }
        "run" | "dev" => {
            let mut input = None;
            for value in flags.rest {
                set_input(&mut input, value)?;
            }
            Ok(if action == "run" {
                Action::Run { input }
            } else {
                Action::Dev { input }
            })
        }
        "functions" | "lint" | "test" | "editor" => Err(unsupported(
            &format!("flush {action}"),
            "the function-mode scripting language",
        )),
        other => Err(usage(&format!("Unknown command: {other}."))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(line: &str) -> Result<Action, CliError> {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        parse(&args)
    }

    fn code(line: &str) -> String {
        run(line).unwrap_err().code
    }

    #[test]
    fn help_and_version() {
        assert_eq!(run("").unwrap(), Action::Help);
        assert_eq!(run("help").unwrap(), Action::Help);
        assert_eq!(run("--help").unwrap(), Action::Help);
        assert_eq!(run("--version").unwrap(), Action::Version);
        assert_eq!(code("help extra"), "USAGE");
        assert_eq!(code("frobnicate"), "USAGE");
    }

    #[test]
    fn init_options() {
        assert_eq!(
            run("init bot --template moderation --local").unwrap(),
            Action::Init {
                directory: "bot".into(),
                template: Some("moderation".into()),
                local: true
            }
        );
        assert_eq!(code("init"), "USAGE");
        assert_eq!(code("init a b"), "USAGE");
        assert_eq!(code("init bot --local --local"), "USAGE");
        assert_eq!(code("init bot --template"), "USAGE");
        assert_eq!(code("init bot --ts"), "UNSUPPORTED");
        assert_eq!(code("init bot --functions"), "UNSUPPORTED");
    }

    #[test]
    fn check_doctor_and_templates() {
        assert_eq!(
            run("check ./cfg.toml --json --build").unwrap(),
            Action::Check {
                input: Some("./cfg.toml".into()),
                json: true,
                build: true
            }
        );
        assert_eq!(
            run("doctor").unwrap(),
            Action::Doctor {
                input: None,
                json: false
            }
        );
        assert_eq!(code("doctor --build"), "USAGE");
        assert_eq!(code("check --json --json"), "USAGE");
        assert_eq!(
            run("templates --json").unwrap(),
            Action::Templates { json: true }
        );
        assert_eq!(code("templates extra"), "USAGE");
    }

    #[test]
    fn data_options() {
        assert_eq!(
            run("data restore data/cases.json --file b.json --apply --policy merge").unwrap(),
            Action::Data {
                operation: DataOperation::Restore,
                input: "data/cases.json".into(),
                file: Some("b.json".into()),
                apply: true,
                policy: RestorePolicy::Merge
            }
        );
        assert_eq!(code("data restore data/cases.json"), "USAGE");
        assert_eq!(code("data stats data/cases.json --apply"), "USAGE");
        assert_eq!(code("data stats"), "USAGE");
        assert_eq!(code("data cleanup data/cases.json"), "UNSUPPORTED");
        assert_eq!(
            code("data restore s.json --file b.json --policy other"),
            "USAGE"
        );
    }

    #[test]
    fn run_and_dev() {
        assert_eq!(run("run").unwrap(), Action::Run { input: None });
        assert_eq!(
            run("dev ./bot").unwrap(),
            Action::Dev {
                input: Some("./bot".into())
            }
        );
        assert_eq!(code("run --shards 4"), "UNSUPPORTED");
        assert_eq!(code("dev --health-port 8080"), "UNSUPPORTED");
        assert_eq!(code("run a b"), "USAGE");
        assert_eq!(code("functions --json"), "UNSUPPORTED");
        assert_eq!(code("lint ./commands"), "UNSUPPORTED");
    }
}
