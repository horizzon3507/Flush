//! Command + context (mirrors `Sour types.ts Command` + `lib/commandReply.ts`).

use std::future::Future;

use flush_core::{Channel, Embed, Guild, MemberUser, Message, MessageSend, Snowflake};
use flush_rest::RestClient;
use thiserror::Error;

use crate::moderation::PermissionAction;
use crate::parse::ParsedCommand;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("{0}")]
    User(String),
    #[error("rest: {0}")]
    Rest(#[from] flush_rest::RestError),
    #[error("internal: {0}")]
    Internal(String),
}

pub type CommandResult = Result<(), CommandError>;

/// Context handed to the handler: message, args, prefix, REST, bot id
/// and read snapshots (guild/channel/member resolved by the router).
pub struct CommandCtx<'a> {
    pub message: &'a Message,
    pub args: Vec<String>,
    pub prefix: String,
    pub rest: &'a RestClient,
    pub bot_id: Option<Snowflake>,
    pub guild: Option<Guild>,
    pub channel: Option<Channel>,
    pub member: Option<MemberUser>,
    pub bot_member: Option<MemberUser>,
    /// Driver-backed voice manager, when the bot attached one (`ctx.voice`).
    pub voice: Option<crate::voice::Voice>,
    /// Shared in-memory voice-state tracker (`ctx.voiceStates`).
    pub voice_states: crate::voice::VoiceStateTracker,
}

impl CommandCtx<'_> {
    pub fn channel_id(&self) -> Snowflake {
        self.message.channel_id
    }

    pub fn guild_id(&self) -> Option<Snowflake> {
        self.message.guild_id
    }

    pub async fn reply_text(&self, content: impl Into<String>) -> Result<Message, CommandError> {
        let payload = MessageSend::text(content);
        Ok(self
            .rest
            .reply(self.channel_id(), self.message, payload, true)
            .await?)
    }

    pub async fn reply_no_ping(&self, content: impl Into<String>) -> Result<Message, CommandError> {
        let payload = MessageSend::text(content);
        Ok(self
            .rest
            .reply(self.channel_id(), self.message, payload, false)
            .await?)
    }

    pub async fn reply_embed(&self, embed: Embed) -> Result<Message, CommandError> {
        let payload = MessageSend {
            content: None,
            embeds: vec![embed],
            ..Default::default()
        };
        Ok(self
            .rest
            .reply(self.channel_id(), self.message, payload, true)
            .await?)
    }

    /// Resolves a member from a mention, `id` or `name!id` via REST.
    /// A 404 becomes `Ok(None)` (the caller decides how to reply).
    pub async fn target_member(&self, raw: &str) -> Result<Option<MemberUser>, CommandError> {
        let Some(guild) = self.guild_id() else {
            return Ok(None);
        };
        let Some(raw) = crate::parse::parse_mention_or_id(raw) else {
            return Ok(None);
        };
        let Ok(id) = raw.trim().parse::<Snowflake>() else {
            return Ok(None);
        };
        match self.rest.member(guild, id).await {
            Ok(m) => Ok(Some(m)),
            Err(flush_rest::RestError::Api { status: 404, .. }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

pub trait Command: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn aliases(&self) -> &[String] {
        &[]
    }
    fn usage(&self) -> Option<&str> {
        None
    }
    fn hidden(&self) -> bool {
        false
    }
    fn cooldown_ms(&self) -> u64 {
        0
    }
    /// Permission required from the executor. `None` = unrestricted.
    /// The router checks it before executing (the server owner passes).
    fn required_permission(&self) -> Option<PermissionAction> {
        None
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>>;
}

/// Sour-style error: `❌ {detail}`.
pub fn plain_error(detail: impl AsRef<str>) -> String {
    format!("❌ {}", detail.as_ref())
}

/// Sour-style info block: `**{title}**\n-# ╰› {line}`.
pub fn info_block(title: impl AsRef<str>, lines: &[String]) -> String {
    let mut out = format!("**{}**", title.as_ref());
    for l in lines {
        out.push_str(&format!("\n-# ╰› {l}"));
    }
    out
}

/// Sour-style moderation success: `{icon} {desc}` + Reason + Case #.
pub fn mod_success(
    icon: &str,
    description: &str,
    reason: &str,
    case_id: u64,
    extra: Option<&str>,
) -> String {
    let mut out = format!("{icon} {description}\n-# ╰› Reason: {reason}");
    if let Some(e) = extra {
        out.push_str(&format!("\n-# ╰› {e}"));
    }
    out.push_str(&format!("\n-# ╰› Case **#{case_id}**"));
    out
}

pub fn parsed_lookup<'a>(
    parsed: &ParsedCommand,
    commands: &'a [Box<dyn Command>],
) -> Option<&'a dyn Command> {
    let lower = parsed.name.to_lowercase();
    commands.iter().find_map(|c| {
        let c: &'a dyn Command = c.as_ref();
        (c.name() == lower || c.aliases().iter().any(|a| a.to_lowercase() == lower)).then_some(c)
    })
}
