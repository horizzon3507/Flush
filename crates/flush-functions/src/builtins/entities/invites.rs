//! yuzuha `native-invites.ts`.

use serde_json::Value;

use crate::errors::codes;
use crate::types::Definitions;

use super::common::{channel_id, err, parse_duration, rest, Res};

/// Invite code from `code`, `fluxer.gg/invite/code` or `fx.gg/code`
/// (yuzuha `inviteCode`: last path segment).
fn invite_code(raw: &str) -> Res<String> {
    let path = raw
        .trim()
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let code = path.rsplit('/').next().unwrap_or(path);
    if code.is_empty() {
        return Err(err(
            codes::INVALID_ARGUMENTS,
            "Missing the invite code or URL.",
        ));
    }
    Ok(code.to_string())
}

pub fn register(d: &mut Definitions) {
    d.add(
        "invite",
        "invites",
        1,
        1,
        "[codigo]",
        "Shows an invite by code or URL.",
        |c| {
            Box::pin(async move {
                let code = invite_code(&c.arg(0).await?)?;
                let invite = rest(c, c.ctx.rest.invite(&code)).await?;
                Ok(super::info::invite_info(&invite))
            })
        },
    );

    d.add(
        "createInvite",
        "invites",
        1,
        3,
        "[canal;maxUsos?;duracao?]",
        "Creates an invite; duration like 1h, 0 never expires.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let max_uses = if c.arg_count() > 1 {
                    let raw = c.arg(1).await?;
                    let raw = raw.trim();
                    if raw.is_empty() {
                        None
                    } else {
                        if !raw.chars().all(|ch| ch.is_ascii_digit()) {
                            return Err(err(
                                codes::INVALID_ARGUMENTS,
                                "Max uses must be an integer greater than or equal to 0.",
                            ));
                        }
                        Some(
                            raw.parse::<u64>()
                                .map_err(|_| err(codes::INVALID_ARGUMENTS, "Invalid max uses."))?,
                        )
                    }
                } else {
                    None
                };
                let max_age = if c.arg_count() > 2 {
                    let raw = c.arg(2).await?;
                    let raw = raw.trim();
                    if raw.is_empty() || raw == "0" {
                        0
                    } else {
                        parse_duration(raw)? / 1000
                    }
                } else {
                    0
                };
                let invite = rest(
                    c,
                    c.ctx.rest.create_invite(
                        channel,
                        &flush_rest::client::InviteCreate {
                            max_age_seconds: Some(max_age),
                            max_uses,
                            temporary: None,
                            unique: None,
                        },
                        None,
                    ),
                )
                .await?;
                Ok(super::info::invite_info(&invite))
            })
        },
    );

    d.add(
        "deleteInvite",
        "invites",
        1,
        1,
        "[codigo]",
        "Deletes an invite by code or URL.",
        |c| {
            Box::pin(async move {
                let code = invite_code(&c.arg(0).await?)?;
                rest(c, c.ctx.rest.delete_invite(&code, None)).await?;
                Ok(Value::String(String::new()))
            })
        },
    );
}
