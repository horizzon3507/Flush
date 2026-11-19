//! `flush init`: writes a new bot project (a Rust crate) into a new directory.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::CliError;
use crate::templates::{self, CommandSpec, Helper, Template};
use crate::VERSION;

/// Minimum Rust version of generated projects (same as the Flush crates).
pub const RUST_REQUIREMENT: &str = env!("CARGO_PKG_RUST_VERSION");

/// Source tree of `flush-lib` next to this CLI's sources, for `--local`.
fn local_library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../flush")
}

fn project_name(root: &Path) -> Result<String, CliError> {
    let name: String = root
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err(CliError::new(
            "INIT_NAME",
            "Pick a directory name that starts with a letter.",
        ));
    }
    Ok(name)
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn commands_module(commands: &[CommandSpec]) -> String {
    let mut modules: Vec<&str> = commands.iter().map(|command| command.module).collect();
    modules.sort_unstable();
    let mut out = String::new();
    for module in modules {
        out.push_str(&format!("pub mod {module};\n"));
    }
    out.push_str("\nuse flush::Bot;\n\n/// Registers every command of this bot.\npub fn register(mut bot: Bot) -> Bot {\n");
    for command in commands {
        out.push_str(&format!(
            "    bot = bot.command({});\n",
            command.constructor
        ));
    }
    out.push_str("    bot\n}\n");
    out
}

fn util_module(commands: &[CommandSpec]) -> Option<String> {
    let uses = |helper| commands.iter().any(|command| command.helper == helper);
    let parts: Vec<&str> = [
        (uses(Helper::Moderation) || uses(Helper::Timeout)).then_some(templates::UTIL_MODERATION),
        uses(Helper::Timeout).then_some(templates::UTIL_TIME),
        uses(Helper::Random).then_some(templates::UTIL_RANDOM),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn readme(
    name: &str,
    template: Option<&Template>,
    commands: &[CommandSpec],
    local: bool,
) -> String {
    let examples: String = commands
        .iter()
        .map(|command| format!("- `!{}`\n", command.example))
        .collect();
    let base = match template {
        Some(template) => format!(
            "Template: **{}**. {}\n\n",
            template.name, template.description
        ),
        None => String::new(),
    };
    let moderation = if commands
        .iter()
        .any(|command| command.helper == Helper::Moderation)
    {
        "\nModeration commands check the author's and the bot's permissions and the role\nhierarchy over the target, and record a case in `data/cases.json`.\n"
    } else {
        ""
    };
    let dependency = if local {
        "The Flush dependency points at the local checkout that generated this project.\nSwitch it to the published crate before sharing the project."
    } else {
        "The Flush dependency uses the published crate, so the project builds on any machine."
    };
    format!(
        "# {name}\n\nA Fluxer bot built with Flush.\n\n\
1. Run `flush check --build` and `cargo test` to validate offline.\n\
2. Copy `.env.example` to `.env` and fill in the token.\n\
3. Run `flush dev` while developing, or `flush run` in production.\n\n\
{base}Commands (`!help` lists them, generated from the registered commands):\n\n{examples}{moderation}\n\
Settings live in `flush.toml`; secrets live in `.env`, which is never committed.\n\
Guild settings and moderation cases are stored as JSON under `data/`.\n\n{dependency}\n"
    )
}

/// Files of a new project, keyed by relative path.
pub fn project_files(
    name: &str,
    template: Option<&Template>,
    local: bool,
) -> BTreeMap<String, String> {
    let commands = template.map_or(templates::DEFAULT_COMMANDS, |template| template.commands);
    let dependency = if local {
        let path = local_library();
        let path = path.canonicalize().unwrap_or(path);
        format!("path = {}", toml_string(&path.to_string_lossy()))
    } else {
        format!("version = \"{VERSION}\"")
    };
    let util = util_module(commands);
    let mut files = BTreeMap::new();
    files.insert(
        "Cargo.toml".to_owned(),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\nrust-version = \"{RUST_REQUIREMENT}\"\npublish = false\n\n\
[dependencies]\nflush = {{ package = \"flush-lib\", {dependency} }}\nasync-std = {{ version = \"1\", features = [\"attributes\"] }}\n\n\
# Keeps this project out of any enclosing Cargo workspace.\n[workspace]\n"
        ),
    );
    files.insert(
        "flush.toml".to_owned(),
        format!(
            "# Flush project config. `flush check` validates it without logging in.\nname = {}\nprefix = \"!\"\ndata_dir = \"data\"\n# presence = [\"!help\"]\n# default_cooldown_ms = 0\n",
            toml_string(name)
        ),
    );
    files.insert(".env.example".to_owned(), "FLUXER_BOT_TOKEN=\n".to_owned());
    files.insert(".gitignore".to_owned(), "/target\n.env\n/data\n".to_owned());
    files.insert(
        "README.md".to_owned(),
        readme(name, template, commands, local),
    );
    files.insert(
        "src/main.rs".to_owned(),
        format!(
            "mod commands;\n{}\nfn main() -> Result<(), Box<dyn std::error::Error>> {{\n    let (bot, config) = flush::bot_from_project(None)?;\n    let mut bot = commands::register(bot).enable_help();\n    println!(\"{{}} is starting.\", config.name);\n    async_std::task::block_on(bot.run())?;\n    Ok(())\n}}\n",
            if util.is_some() { "mod util;\n" } else { "" }
        ),
    );
    files.insert("src/commands/mod.rs".to_owned(), commands_module(commands));
    for command in commands {
        files.insert(
            format!("src/commands/{}.rs", command.module),
            command.source.to_owned(),
        );
    }
    if let Some(util) = util {
        files.insert("src/util.rs".to_owned(), util);
    }
    files
}

/// Creates the project. The directory must not exist; a failure midway
/// removes what was written.
pub fn scaffold(
    directory: &Path,
    template: Option<&str>,
    local: bool,
) -> Result<PathBuf, CliError> {
    let template = match template {
        Some(name) => Some(templates::find(name).ok_or_else(|| {
            CliError::new(
                "INIT_TEMPLATE",
                format!("Pick a template: {}.", templates::names()),
            )
        })?),
        None => None,
    };
    let root = std::path::absolute(directory)?;
    let name = project_name(&root)?;
    if local && !local_library().join("Cargo.toml").is_file() {
        return Err(CliError::new(
            "INIT_LOCAL",
            "--local needs the Flush checkout this CLI was built from; it is no longer there.",
        ));
    }
    let files = project_files(&name, template, local);
    std::fs::create_dir(&root).map_err(|e| {
        CliError::new(
            "INIT_DIRECTORY",
            format!(
                "Cannot create {}: {e}. The directory must be new.",
                root.display()
            ),
        )
    })?;
    let write = || -> std::io::Result<()> {
        for (file, content) in &files {
            let path = root.join(file);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?
                .write_all(content.as_bytes())?;
        }
        Ok(())
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_dir_all(&root);
        return Err(error.into());
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scaffold_has_ping_hello_and_ban() {
        let files = project_files("demo", None, false);
        for file in [
            "Cargo.toml",
            "flush.toml",
            ".env.example",
            ".gitignore",
            "README.md",
            "src/main.rs",
            "src/commands/mod.rs",
            "src/commands/ping.rs",
            "src/commands/hello.rs",
            "src/commands/ban.rs",
            "src/util.rs",
        ] {
            assert!(files.contains_key(file), "missing {file}");
        }
        assert!(files["Cargo.toml"].contains(&format!("version = \"{VERSION}\" }}")));
        assert!(files["src/main.rs"].contains("mod util;"));
        let config = flush_framework::ProjectConfig::parse(&files["flush.toml"]).unwrap();
        assert_eq!(config.name, "demo");
    }

    #[test]
    fn templates_generate_their_commands() {
        for template in templates::TEMPLATES {
            let files = project_files("demo", Some(template), true);
            for command in template.commands {
                assert!(files.contains_key(&format!("src/commands/{}.rs", command.module)));
                assert!(files["src/commands/mod.rs"].contains(command.constructor));
            }
            assert!(files["Cargo.toml"].contains("path = "));
        }
        let simple = project_files("demo", templates::find("simple"), false);
        assert!(!simple.contains_key("src/util.rs"));
        assert!(!simple["src/main.rs"].contains("mod util;"));
    }

    #[test]
    fn rejects_bad_names_templates_and_existing_directories() {
        let base = std::env::temp_dir().join(format!("flush-scaffold-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(
            scaffold(&base.join("9bot"), None, false).unwrap_err().code,
            "INIT_NAME"
        );
        assert_eq!(
            scaffold(&base.join("bot"), Some("nope"), false)
                .unwrap_err()
                .code,
            "INIT_TEMPLATE"
        );
        let root = scaffold(&base.join("My Bot"), None, false).unwrap();
        assert!(std::fs::read_to_string(root.join("Cargo.toml"))
            .unwrap()
            .contains("name = \"my-bot\""));
        assert_eq!(
            scaffold(&base.join("My Bot"), None, false)
                .unwrap_err()
                .code,
            "INIT_DIRECTORY"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }
}
