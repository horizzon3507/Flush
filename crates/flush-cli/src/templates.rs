//! Bot templates: which command files each one generates.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Helper {
    None,
    Random,
    Moderation,
    /// Moderation helpers plus timestamps.
    Timeout,
}

/// One generated command file.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    /// Catalog path, as listed by `flush templates`.
    pub path: &'static str,
    /// Module name under `src/commands/`.
    pub module: &'static str,
    /// Expression that builds the command.
    pub constructor: &'static str,
    pub source: &'static str,
    pub helper: Helper,
    /// Example invocation for the generated README.
    pub example: &'static str,
}

macro_rules! command {
    ($path:literal, $module:literal, $ctor:literal, $helper:ident, $example:literal) => {
        CommandSpec {
            path: $path,
            module: $module,
            constructor: $ctor,
            source: include_str!(concat!("../templates/commands/", $module, ".rs")),
            helper: Helper::$helper,
            example: $example,
        }
    };
}

const PING: CommandSpec = command!("general/ping", "ping", "ping::Ping", None, "ping");
const HELLO: CommandSpec = command!(
    "general/hello",
    "hello",
    "hello::Hello",
    None,
    "hello Flush"
);
const ABOUT: CommandSpec = command!("general/about", "about", "about::About", None, "about");
const AVATAR: CommandSpec = command!(
    "utilities/avatar",
    "avatar",
    "avatar::Avatar",
    None,
    "avatar @member"
);
const BAN: CommandSpec = command!(
    "moderation/ban",
    "ban",
    "ban::Ban",
    Moderation,
    "ban @member spam"
);
const KICK: CommandSpec = command!(
    "moderation/kick",
    "kick",
    "kick::Kick",
    Moderation,
    "kick @member spam"
);
const TIMEOUT: CommandSpec = command!(
    "moderation/timeout",
    "timeout",
    "timeout::Timeout",
    Timeout,
    "timeout @member 10m spam"
);
const UNBAN: CommandSpec = command!(
    "moderation/unban",
    "unban",
    "unban::Unban",
    Moderation,
    "unban 123456789 appeal accepted"
);
const CASE: CommandSpec = command!("moderation/case", "case", "case::Case", None, "case 1");
const DICE: CommandSpec = command!("fun/dice", "dice", "dice::Dice", Random, "dice 20");
const COIN: CommandSpec = command!("fun/coin", "coin", "coin::Coin::default()", Random, "coin");
const CHOOSE: CommandSpec = command!(
    "fun/choose",
    "choose",
    "choose::Choose",
    Random,
    "choose pizza | sushi | pasta"
);
const EIGHTBALL: CommandSpec = command!(
    "fun/eightball",
    "eightball",
    "eightball::EightBall::default()",
    Random,
    "eightball Will it rain?"
);

/// Commands of the scaffold generated without `--template`.
pub const DEFAULT_COMMANDS: &[CommandSpec] = &[PING, HELLO, BAN];

#[derive(Debug, Clone, Copy)]
pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub commands: &'static [CommandSpec],
}

pub const TEMPLATES: &[Template] = &[
    Template {
        name: "simple",
        description: "Small bot with help, about, avatar and ping.",
        commands: &[PING, ABOUT, AVATAR],
    },
    Template {
        name: "moderation",
        description:
            "Ban, kick, timeout, unban and case lookup with shared permission and hierarchy checks.",
        commands: &[PING, ABOUT, AVATAR, BAN, KICK, TIMEOUT, UNBAN, CASE],
    },
    Template {
        name: "entertainment",
        description: "Dice, coin flip, pick between options and an 8-ball.",
        commands: &[PING, ABOUT, AVATAR, DICE, COIN, CHOOSE, EIGHTBALL],
    },
];

pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|template| template.name == name)
}

pub fn names() -> String {
    TEMPLATES
        .iter()
        .map(|template| template.name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Serialize)]
pub struct TemplateInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub commands: Vec<&'static str>,
}

pub fn catalog() -> Vec<TemplateInfo> {
    TEMPLATES
        .iter()
        .map(|template| TemplateInfo {
            name: template.name,
            description: template.description,
            commands: template
                .commands
                .iter()
                .map(|command| command.path)
                .collect(),
        })
        .collect()
}

pub const UTIL_RANDOM: &str = include_str!("../templates/util_random.rs");
pub const UTIL_MODERATION: &str = include_str!("../templates/util_moderation.rs");
pub const UTIL_TIME: &str = include_str!("../templates/util_time.rs");
