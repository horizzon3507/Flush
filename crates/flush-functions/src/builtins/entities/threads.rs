//! yuzuha `native-threads.ts` + thread builtins of `entities-channels.ts`
//! (`editThreadDefaults`, `editThreadSelf`, `lockThread`).

use serde_json::{json, Value};

use flush_core::message::MessageSend;
use flush_core::thread::StartThread;
use flush_core::{ChannelType, Permissions};

use crate::errors::codes;
use crate::types::Definitions;

use super::common::{
    arg_value, channel_id, context_channel, err, integer, opt, require_perms, rest, serialize,
    snowflake, user_id, Res,
};

async fn opt_arg(c: &crate::call::ScriptCall<'_>, i: usize) -> Res<Option<String>> {
    if i >= c.arg_count() {
        return Ok(None);
    }
    Ok(opt(c.arg(i).await?))
}

async fn thread_member_args(
    c: &crate::call::ScriptCall<'_>,
) -> Res<(flush_core::Snowflake, flush_core::Snowflake)> {
    let raw = c.arg(0).await?;
    if raw.trim().is_empty() {
        return Err(err("THREAD_ID", "The thread ID is required."));
    }
    let thread = channel_id(&raw)?;
    let user = user_id(&c.arg(1).await?)?;
    Ok((thread, user))
}

fn thread_name(raw: &str) -> Res<String> {
    let name = raw.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(err(
            codes::FUNCTION_ERROR,
            "Thread name must be 1 to 100 characters.",
        ));
    }
    Ok(name.to_string())
}

fn archive_minutes(raw: &str) -> Res<u32> {
    let n = integer(raw)?;
    if ![60, 1440, 4320, 10080].contains(&n) {
        return Err(err(
            codes::FUNCTION_ERROR,
            format!("Invalid duration: {n}."),
        ));
    }
    Ok(n as u32)
}

pub fn register(d: &mut Definitions) {
    d.add(
        "activeThreads",
        "threads",
        0,
        0,
        "",
        "Lists the guild's active threads and their associations.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let listing = rest(c, c.ctx.rest.active_threads(guild)).await?;
                Ok(serialize(&serde_json::json!({
                    "threads": listing.threads,
                    "members": listing.members,
                    "hasMore": listing.has_more,
                })))
            })
        },
    );

    d.add(
        "archivedThreads",
        "threads",
        1,
        4,
        "[canal;tipo?;limite?;antes?]",
        "Pages archived threads: public/private use an ISO timestamp; joined uses an ID.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let kind = opt_arg(c, 1).await?.unwrap_or_else(|| "public".into());
                let limit = opt_arg(c, 2).await?.map(|s| integer(&s)).transpose()?;
                let before = opt_arg(c, 3).await?;
                let private = match kind.as_str() {
                    "public" => false,
                    "private" | "joined" => true,
                    other => {
                        return Err(err(
                            codes::FUNCTION_ERROR,
                            format!("Invalid thread type: {other}."),
                        ))
                    }
                };
                let _ = context_channel(c, Some(channel)).await?;
                let listing = rest(
                    c,
                    c.ctx.rest.archived_threads(
                        channel,
                        private,
                        before.as_deref(),
                        limit.map(|n| n as u32),
                    ),
                )
                .await?;
                Ok(serialize(&serde_json::json!({
                    "threads": listing.threads,
                    "members": listing.members,
                    "hasMore": listing.has_more,
                })))
            })
        },
    );

    d.add(
        "threadMembers",
        "threads",
        1,
        3,
        "[thread;limite?;após?]",
        "Lists members of a thread with a limit of 1 to 100 and a user cursor.",
        |c| {
            Box::pin(async move {
                let thread = channel_id(&c.arg(0).await?)?;
                let limit = match opt_arg(c, 1).await? {
                    Some(s) => {
                        let n = integer(&s)?;
                        if !(1..=100).contains(&n) {
                            return Err(err(
                                codes::FUNCTION_ERROR,
                                format!("Invalid limit: {n}."),
                            ));
                        }
                        Some(n as u32)
                    }
                    None => None,
                };
                let after = opt_arg(c, 2)
                    .await?
                    .map(|s| snowflake(&s, "member"))
                    .transpose()?;
                if after.is_some() || limit.is_some() {
                    let value = rest(
                        c,
                        c.ctx.rest.thread_members_page(thread, limit.unwrap_or(100), after),
                    )
                    .await?;
                    return Ok(value);
                }
                let members = rest(c, c.ctx.rest.thread_members(thread)).await?;
                Ok(Value::Array(
                    members
                        .iter()
                        .map(|m| super::common::camelize(&serde_json::json!({
                            "id": m.id,
                            "user_id": m.user_id,
                            "join_timestamp": m.join_timestamp,
                            "flags": m.flags,
                            "muted": m.muted,
                            "member": m.member.as_ref().map(|mb| super::info::member_info(mb, m.user_id.unwrap_or_default())),
                        })))
                        .collect(),
                ))
            })
        },
    );

    d.add(
        "threadMember",
        "threads",
        1,
        2,
        "[thread;usuário?]",
        "Fetches a thread member with flags and mute state; defaults to the author.",
        |c| {
            Box::pin(async move {
                let thread = channel_id(&c.arg(0).await?)?;
                let user = opt_arg(c, 1)
                    .await?
                    .map(|s| user_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.author_id());
                let member = rest(c, c.ctx.rest.thread_member(thread, user)).await?;
                Ok(super::common::camelize(&serialize(&member)))
            })
        },
    );

    d.add(
        "createThread",
        "threads",
        1,
        4,
        "[nome;canal?;mensagem?;arquivamento?]",
        "Creates a thread in the current or given channel.",
        |c| {
            Box::pin(async move {
                let name = thread_name(&c.arg(0).await?)?;
                let channel = opt_arg(c, 1)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let message = opt_arg(c, 2)
                    .await?
                    .map(|s| snowflake(&s, "message"))
                    .transpose()?;
                let minutes = opt_arg(c, 3)
                    .await?
                    .map(|s| archive_minutes(&s))
                    .transpose()?;
                let parent = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&parent), Permissions::CREATE_PUBLIC_THREADS).await?;
                let start = StartThread {
                    name,
                    auto_archive_duration: minutes,
                    ..Default::default()
                };
                let thread = match message {
                    Some(message) => {
                        rest(
                            c,
                            c.ctx
                                .rest
                                .start_thread_from_message(channel, message, &start, None),
                        )
                        .await?
                    }
                    None => rest(c, c.ctx.rest.start_thread(channel, &start, None)).await?,
                };
                Ok(super::info::channel_thread_info(&thread))
            })
        },
    );

    d.add(
        "archiveThread",
        "threads",
        1,
        2,
        "[thread;bloquear?]",
        "Archives a thread; lock accepts true/false.",
        |c| {
            Box::pin(async move {
                let thread = channel_id(&c.arg(0).await?)?;
                let locked = opt_arg(c, 1)
                    .await?
                    .map(|s| super::common::boolean(&s))
                    .transpose()?;
                rest(
                    c,
                    c.ctx
                        .rest
                        .set_thread_state(thread, Some(true), locked, None),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "unarchiveThread",
        "threads",
        1,
        1,
        "[thread]",
        "Unarchives a thread.",
        |c| {
            Box::pin(async move {
                let thread = channel_id(&c.arg(0).await?)?;
                rest(
                    c,
                    c.ctx.rest.set_thread_state(thread, Some(false), None, None),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "forumPost",
        "threads",
        3,
        4,
        "[canal;título;conteúdo;tags?]",
        "Creates a forum/media post; comma-separated tags.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let title = thread_name(&c.arg(1).await?)?;
                let content = c.arg(2).await?;
                let tags = opt_arg(c, 3).await?;
                let parent = context_channel(c, Some(channel)).await?;
                if parent.kind != ChannelType::GuildForum && parent.kind != ChannelType::GuildMedia
                {
                    return Err(err(
                        codes::FUNCTION_ERROR,
                        "Forum posts require a forum or media channel.",
                    ));
                }
                require_perms(c, Some(&parent), Permissions::SEND_MESSAGES).await?;
                let applied_tags = tags
                    .map(|t| {
                        t.split(',')
                            .map(|tag| snowflake(tag.trim(), "tag"))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?;
                let start = StartThread {
                    name: title,
                    applied_tags,
                    message: Some(MessageSend {
                        content: Some(content),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                let thread = rest(c, c.ctx.rest.start_thread(channel, &start, None)).await?;
                Ok(super::info::channel_thread_info(&thread))
            })
        },
    );

    // ---- entities-channels.ts ----

    d.add(
        "editThreadDefaults",
        "threads",
        2,
        2,
        "[canal;opções]",
        "Sets default autoArchiveDuration and slowmode; null clears. Requires ManageChannels of author and bot.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let options = arg_value(c, 1).await?;
                let target = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                let Some(map) = options.as_object() else {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected an options object."));
                };
                let mut body = json!({});
                for key in ["autoArchiveDuration", "slowmode"] {
                    if let Some(value) = map.get(key) {
                        body[key] = value.clone();
                    }
                }
                rest(c, c.ctx.rest.patch_channel(channel, body, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "editThreadSelf",
        "threads",
        2,
        2,
        "[thread;opções]",
        "Edits only the bot's association: flags, muted and muteConfig. Requires ViewChannel of author and bot.",
        |c| {
            Box::pin(async move {
                let thread = channel_id(&c.arg(0).await?)?;
                let options = arg_value(c, 1).await?;
                let target = context_channel(c, Some(thread)).await?;
                require_perms(c, Some(&target), Permissions::VIEW_CHANNEL).await?;
                let Some(map) = options.as_object() else {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected an options object."));
                };
                let settings = flush_rest::account::ThreadMemberSettings {
                    flags: map.get("flags").and_then(|v| v.as_u64()),
                    muted: map.get("muted").and_then(|v| v.as_bool()),
                    mute_config: map.get("muteConfig").map(|v| {
                        Some(flush_rest::account::ThreadMuteConfig {
                            end_time: v.get("end_time").and_then(|e| {
                                if e.is_null() { Some(None) } else { e.as_str().map(|s| Some(s.to_string())) }
                            }),
                            selected_time_window: v.get("selected_time_window").and_then(|n| n.as_i64()).unwrap_or(0),
                        })
                    }),
                };
                rest(
                    c,
                    c.ctx.rest.edit_thread_member_settings(thread, &settings),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "lockThread",
        "threads",
        2,
        2,
        "[thread;bloquear]",
        "Locks or unlocks a thread with true/false and ManageChannels of author and bot.",
        |c| {
            Box::pin(async move {
                let thread = channel_id(&c.arg(0).await?)?;
                let locked = super::common::boolean(&c.arg(1).await?)?;
                let target = context_channel(c, Some(thread)).await?;
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                rest(
                    c,
                    c.ctx
                        .rest
                        .set_thread_state(thread, None, Some(locked), None),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "addThreadMember",
        "threads",
        2,
        2,
        "[thread;usuário]",
        "Adds a member to the thread.",
        |c| {
            Box::pin(async move {
                let (thread, user) = thread_member_args(&c).await?;
                rest(c, c.ctx.rest.add_thread_member(thread, user)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "removeThreadMember",
        "threads",
        2,
        2,
        "[thread;usuário]",
        "Removes a member from the thread.",
        |c| {
            Box::pin(async move {
                let (thread, user) = thread_member_args(&c).await?;
                rest(c, c.ctx.rest.remove_thread_member(thread, user)).await?;
                Ok(Value::Null)
            })
        },
    );
}
