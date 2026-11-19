use flush_core::events::DispatchEnvelope;
use flush_core::{
    Channel, ChannelType, Event, Guild, Message, MessageSend, MessageType, Role, Snowflake,
};

fn msg_fixture(embeds_null: bool) -> serde_json::Value {
    let embeds = if embeds_null {
        serde_json::Value::Null
    } else {
        serde_json::json!([])
    };
    serde_json::json!({
        "id": "1501320000000000000",
        "channel_id": "1501314428688998182",
        "author": {
            "id": "1501314428688990000",
            "username": "ada",
            "discriminator": "0001",
            "avatar": null,
            "avatar_color": null,
            "flags": 0
        },
        "type": 19,
        "content": "oi",
        "timestamp": "2026-08-31T09:14:02.000Z",
        "pinned": null,
        "mention_everyone": null,
        "mentions": null,
        "mention_roles": null,
        "embeds": embeds,
        "attachments": null,
        "reactions": null
    })
}

#[test]
fn message_type_numeric_reply() {
    let m: Message = serde_json::from_value(msg_fixture(false)).unwrap();
    assert_eq!(m.kind, MessageType::Reply);
    assert!(m.mentions.is_empty());
    assert!(!m.pinned);
}

#[test]
fn message_null_lists_become_empty() {
    let m: Message = serde_json::from_value(msg_fixture(true)).unwrap();
    assert!(m.embeds.is_empty());
    assert!(m.attachments.is_empty());
    assert!(m.reactions.is_empty());
}

#[test]
fn message_type_unknown_maps_to_unknown() {
    let mut v = msg_fixture(false);
    v["type"] = serde_json::json!(200);
    let m: Message = serde_json::from_value(v).unwrap();
    assert_eq!(m.kind, MessageType::Unknown);
}

#[test]
fn channel_type_numeric_and_unknown() {
    let ch: Channel = serde_json::from_value(serde_json::json!({
        "id": "1", "type": 0, "permission_overwrites": null, "recipients": null
    }))
    .unwrap();
    assert_eq!(ch.kind, ChannelType::GuildText);

    let ch: Channel = serde_json::from_value(serde_json::json!({
        "id": "1", "type": 7777, "permission_overwrites": null, "recipients": null
    }))
    .unwrap();
    assert_eq!(ch.kind, ChannelType::Unknown);
}

#[test]
fn role_permissions_string_and_int() {
    let r: Role = serde_json::from_value(serde_json::json!({
        "id": "10", "name": "Mod", "permissions": "1099511637006"
    }))
    .unwrap();
    assert_eq!(r.permissions, 1099511637006u64);

    let r: Role = serde_json::from_value(serde_json::json!({
        "id": "10", "name": "Mod", "permissions": 8
    }))
    .unwrap();
    assert_eq!(r.permissions, 8u64);
}

#[test]
fn guild_nested_snapshot_decodes() {
    let g: Guild = serde_json::from_value(serde_json::json!({
        "id": "100",
        "properties": {
            "id": "100",
            "name": "Teste",
            "owner_id": "1",
            "roles": [{ "id": "5", "name": "Mod", "permissions": "8" }]
        },
        "roles": null,
        "member_count": 42
    }))
    .unwrap();
    assert_eq!(g.name, "Teste");
    assert_eq!(g.roles.len(), 1);
    assert_eq!(g.member_count, Some(42));
}

#[test]
fn guild_flat_decodes() {
    let g: Guild = serde_json::from_value(serde_json::json!({
        "id": "100", "name": "Flat", "owner_id": "1", "roles": null
    }))
    .unwrap();
    assert_eq!(g.name, "Flat");
    assert!(g.roles.is_empty());
}

#[test]
fn ready_with_null_lists() {
    let env: DispatchEnvelope = serde_json::from_value(serde_json::json!({
        "op": 0, "t": "READY", "s": 1,
        "d": { "session_id": "abc", "guilds": null, "users": null }
    }))
    .unwrap();
    let ev = Event::decode(&env);
    match ev {
        Event::Ready(p) => {
            assert_eq!(p.session_id, "abc");
            assert!(p.guilds.is_empty());
        }
        other => panic!("expected Ready, got {}", other.kind()),
    }
}

#[test]
fn presence_user_id_only() {
    let env: DispatchEnvelope = serde_json::from_value(serde_json::json!({
        "op": 0, "t": "PRESENCE_UPDATE", "s": 9,
        "d": { "user": { "id": "42" }, "guild_id": "100", "status": "online" }
    }))
    .unwrap();
    let ev = Event::decode(&env);
    assert_eq!(ev.kind(), "PRESENCE_UPDATE");
}

#[test]
fn message_create_dispatch_decodes() {
    let env: DispatchEnvelope = serde_json::from_value(serde_json::json!({
        "op": 0, "t": "MESSAGE_CREATE", "s": 17, "d": msg_fixture(false)
    }))
    .unwrap();
    let ev = Event::decode(&env);
    assert_eq!(ev.kind(), "MESSAGE_CREATE");
}

#[test]
fn invite_tolerates_null_fields() {
    let invite: flush_core::Invite = serde_json::from_value(serde_json::json!({
        "code": "flush%2Fabc",
        "uses": 12,
        "max_uses": null,
        "temporary": true,
        "created_at": null,
        "inviter": { "id": "9", "username": "ada" },
        "guild": null,
        "channel": null
    }))
    .unwrap();
    assert_eq!(invite.code, "flush%2Fabc");
    assert_eq!(invite.uses, 12);
    assert!(invite.temporary);
    assert_eq!(
        invite.inviter.as_ref().map(|u| u.id.to_string()).as_deref(),
        Some("9")
    );
    assert!(invite.guild.is_none());
}

#[test]
fn reaction_emoji_routes_encoding() {
    use flush_core::ReactionEmoji;
    // unicode → percent bytes; custom → name:id with `:` escaped.
    let unicode = ReactionEmoji {
        id: None,
        name: "🎉".into(),
    };
    assert_eq!(unicode.escaped(), "🎉");
    assert_eq!(unicode.encoded(), "%F0%9F%8E%89");
    let custom = ReactionEmoji {
        id: Some("77".parse().unwrap()),
        name: "star".into(),
    };
    assert_eq!(custom.escaped(), "star:77");
    assert_eq!(custom.encoded(), "star%3A77");
}

#[test]
fn message_send_no_ping_text() {
    let send = flush_core::MessageSend::no_ping_text("log");
    assert_eq!(send.content.as_deref(), Some("log"));
    assert!(send.allowed_mentions.as_ref().unwrap().replied_user == Some(false));
    assert!(send.validate().is_ok());
}

fn dispatch(kind: &str, data: serde_json::Value) -> Event {
    Event::decode(&DispatchEnvelope {
        kind: kind.into(),
        seq: 1,
        data,
    })
}

#[test]
fn dispatch_table_matches_sdk_size() {
    assert_eq!(flush_core::DISPATCH_EVENTS.len(), 80);
    assert!(flush_core::is_known_dispatch("THREAD_LIST_SYNC"));
    assert!(!flush_core::is_known_dispatch("NOT_AN_EVENT"));
}

#[test]
fn thread_events_decode() {
    let ev = dispatch(
        "THREAD_CREATE",
        serde_json::json!({
            "id": "5", "type": 11, "guild_id": "1", "parent_id": "2", "name": "t",
            "thread_metadata": { "archived": false, "auto_archive_duration": 1440, "locked": null },
            "applied_tags": null
        }),
    );
    match ev {
        Event::ThreadCreate(ch) => {
            assert!(ch.is_thread());
            assert_eq!(ch.thread_metadata.unwrap().auto_archive_duration, 1440);
        }
        other => panic!("unexpected {other:?}"),
    }
    let ev = dispatch(
        "THREAD_MEMBERS_UPDATE",
        serde_json::json!({ "id": "5", "guild_id": "1", "member_count": 3,
            "added_members": null, "removed_member_ids": ["9"] }),
    );
    assert!(matches!(
        ev,
        Event::ThreadMembersUpdate { member_count: 3, ref removed_member_ids, .. }
            if removed_member_ids.len() == 1
    ));
}

#[test]
fn expression_webhook_voice_events_decode() {
    let ev = dispatch(
        "GUILD_EMOJIS_UPDATE",
        serde_json::json!({ "guild_id": "1", "emojis": [{ "id": "7", "name": "ok", "animated": true }] }),
    );
    match ev {
        Event::GuildEmojisUpdate { emojis, .. } => assert_eq!(emojis[0].mention(), "<a:ok:7>"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        dispatch(
            "WEBHOOKS_UPDATE",
            serde_json::json!({ "guild_id": "1", "channel_id": "2" })
        )
        .kind(),
        "WEBHOOKS_UPDATE"
    );
    let ev = dispatch(
        "VOICE_STATE_UPDATE",
        serde_json::json!({ "user_id": "3", "channel_id": null, "mute": null, "deaf": false }),
    );
    assert!(matches!(ev, Event::VoiceStateUpdate(ref v) if v.channel_id.is_none()));
    let ev = dispatch(
        "GUILD_AUDIT_LOG_ENTRY_CREATE",
        serde_json::json!({ "guild_id": "1", "id": "8", "action_type": 22, "changes": null }),
    );
    assert!(matches!(
        ev,
        Event::GuildAuditLogEntryCreate { ref entry, .. }
            if entry.action_type == flush_core::AuditLogAction::MemberBanAdd
    ));
}

#[test]
fn mismatched_or_unknown_event_keeps_raw_payload() {
    let ev = dispatch("NOT_AN_EVENT", serde_json::json!({ "x": 1 }));
    assert_eq!(ev.wire_name(), "NOT_AN_EVENT");
    assert!(matches!(ev, Event::Unknown { .. }));
    let bad = dispatch("THREAD_DELETE", serde_json::json!({}));
    assert!(matches!(bad, Event::Unknown { .. }));
    let bad = dispatch("MESSAGE_ACK", serde_json::json!({ "channel_id": "1" }));
    assert_eq!(bad.wire_name(), "MESSAGE_ACK");
    assert!(matches!(bad, Event::Unknown { .. }));
}

#[test]
fn every_dispatch_name_has_a_typed_variant() {
    use serde_json::json;
    let user = json!({ "id": "1", "username": "a" });
    let message = json!({ "id": "5", "channel_id": "2", "author": user });
    let mut failed = Vec::new();
    for name in flush_core::DISPATCH_EVENTS {
        let data = match *name {
            "READY" => json!({ "session_id": "s" }),
            "MESSAGE_CREATE" | "MESSAGE_UPDATE" | "SAVED_MESSAGE_CREATE" => message.clone(),
            "WEBAUTHN_CREDENTIALS_UPDATE" => json!([]),
            "USER_UPDATE" => user.clone(),
            _ => json!({
                "id": "1", "guild_id": "3", "channel_id": "2", "message_id": "5",
                "user_id": "1", "thread_id": "9", "sound_id": "8", "role_id": "4",
                "type": 0, "name": "n", "code": "abc", "user": user, "token": "t",
                "role": { "id": "4", "name": "r", "color": 0, "position": 0, "permissions": "0",
                          "hoist": false, "mentionable": false },
                "action_type": 1,
                "emoji": { "name": "x" },
                "member": { "user": user },
            }),
        };
        let ev = dispatch(name, data);
        if matches!(ev, Event::Unknown { .. }) || ev.kind() != *name {
            failed.push(*name);
        }
    }
    assert!(failed.is_empty(), "decoded as Unknown: {failed:?}");
}

#[test]
fn read_state_and_count_events_decode() {
    use serde_json::json;
    assert_eq!(
        dispatch(
            "MESSAGE_ACK",
            json!({ "channel_id": "2", "message_id": "5" })
        ),
        Event::MessageAck {
            channel_id: Snowflake(2),
            message_id: Snowflake(5)
        }
    );
    let ev = dispatch(
        "GUILD_COUNTS_UPDATE",
        json!({ "counts": [{ "guild_id": "3", "member_count": 10, "online_count": null }] }),
    );
    match ev {
        Event::GuildCountsUpdate { counts } => {
            assert_eq!(counts[0].member_count, 10);
            assert_eq!(counts[0].online_count, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        dispatch("CHANNEL_MEMBER_COUNTS_UPDATE", json!({ "counts": null })),
        Event::ChannelMemberCountsUpdate { counts: vec![] }
    );
    let ev = dispatch(
        "FORUM_UNREADS",
        json!({ "guild_id": "3", "channel_id": "2",
                "threads": [{ "thread_id": "9", "count": 4 }, { "thread_id": "10", "missing": true }] }),
    );
    match ev {
        Event::ForumUnreads { threads, .. } => {
            assert_eq!(threads[0].count, Some(4));
            assert_eq!(threads[1].missing, Some(true));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn bulk_reaction_presence_and_member_list_events_decode() {
    use serde_json::json;
    let ev = dispatch(
        "MESSAGE_REACTION_ADD_MANY",
        json!({ "channel_id": "2", "message_id": "5",
                "reactions": [{ "user_id": "1", "emoji": { "name": "x" }, "member": { "nick": "n" } }] }),
    );
    match ev {
        Event::MessageReactionAddMany {
            reactions,
            guild_id,
            ..
        } => {
            assert_eq!(guild_id, None);
            assert_eq!(reactions[0].user_id, Snowflake(1));
            assert_eq!(reactions[0].member.as_ref().unwrap()["nick"], "n");
        }
        other => panic!("{other:?}"),
    }
    let ev = dispatch(
        "PRESENCE_UPDATE_BULK",
        json!({ "guild_id": "3", "presences": [{ "user": { "id": "1" }, "status": "online" }] }),
    );
    match ev {
        Event::PresenceUpdateBulk { presences, .. } => {
            assert_eq!(presences[0].status.as_deref(), Some("online"))
        }
        other => panic!("{other:?}"),
    }
    let ev = dispatch(
        "GUILD_MEMBER_LIST_UPDATE",
        json!({ "guild_id": "3", "id": "everyone", "member_count": 2, "online_count": 1,
                "groups": [{ "id": "online", "count": 1 }],
                "ops": [{ "op": "SYNC", "range": [0, 99], "items": [{ "group": { "id": "online" } }] },
                        { "op": "DELETE", "index": 3 }] }),
    );
    match ev {
        Event::GuildMemberListUpdate(u) => {
            assert_eq!(u.ops[0].range, Some((0, 99)));
            assert_eq!(u.ops[0].items.len(), 1);
            assert_eq!(u.ops[1].index, Some(3));
            assert!(u.ops[1].items.is_empty());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn account_events_decode() {
    use flush_core::RelationshipType;
    use serde_json::json;
    match dispatch("RELATIONSHIP_ADD", json!({ "id": "1", "type": 3 })) {
        Event::RelationshipAdd(r) => assert_eq!(r.kind, RelationshipType::IncomingRequest),
        other => panic!("{other:?}"),
    }
    match dispatch("RELATIONSHIP_UPDATE", json!({ "id": "1", "type": 77 })) {
        Event::RelationshipUpdate(r) => assert_eq!(r.kind, RelationshipType::Unknown),
        other => panic!("{other:?}"),
    }
    match dispatch(
        "USER_SETTINGS_UPDATE",
        json!({ "locale": "en-US", "custom_status": { "text": "hi" }, "developer_mode": true }),
    ) {
        Event::UserSettingsUpdate(s) => {
            assert_eq!(s.locale.as_deref(), Some("en-US"));
            assert_eq!(s.custom_status.unwrap().text.as_deref(), Some("hi"));
            assert_eq!(s.extra["developer_mode"], true);
        }
        other => panic!("{other:?}"),
    }
    match dispatch(
        "SESSIONS_REPLACE",
        json!([{ "session_id": "s", "status": "idle" }]),
    ) {
        Event::SessionsReplace { sessions } => {
            assert_eq!(sessions[0].status.as_deref(), Some("idle"))
        }
        other => panic!("{other:?}"),
    }
    match dispatch(
        "WEBAUTHN_CREDENTIALS_UPDATE",
        json!([{ "id": "c", "name": "key", "created_at": "t", "last_used_at": null }]),
    ) {
        Event::WebAuthnCredentialsUpdate(c) => assert_eq!(c[0].name, "key"),
        other => panic!("{other:?}"),
    }
    match dispatch(
        "VOICE_STATE_ACK",
        json!({ "mutation_id": "m", "status": "applied",
                "canonical_state": { "user_id": "1", "channel_id": "2" } }),
    ) {
        Event::VoiceStateAck(a) => {
            assert_eq!(a.canonical_state.unwrap().channel_id, Some(Snowflake(2)))
        }
        other => panic!("{other:?}"),
    }
    match dispatch(
        "CALL_CREATE",
        json!({ "id": "1", "channel_id": "2", "ringing": ["3"] }),
    ) {
        Event::CallCreate(c) => assert_eq!(c.extra["ringing"][0], "3"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn message_send_attachments_serialize() {
    use flush_core::AttachmentRef;
    let payload = MessageSend {
        attachments: vec![AttachmentRef {
            id: 0,
            filename: "a.png".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(payload.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&payload).unwrap(),
        serde_json::json!({ "attachments": [{ "id": 0, "filename": "a.png" }] })
    );
    assert!(MessageSend::default().validate().is_err());
}

#[test]
fn channel_types_and_cdn_follow_sdk() {
    let ch: flush_core::Channel = serde_json::from_value(
        serde_json::json!({ "id": "1", "type": 15, "available_tags": null }),
    )
    .unwrap();
    assert!(ch.is_forum_like() && !ch.is_text_bearing());
    let sf = flush_core::Snowflake(42);
    assert_eq!(
        flush_core::emoji_url(sf, false, Some(1)),
        "https://fluxerusercontent.com/emojis/42.png?size=32"
    );
    assert_eq!(
        flush_core::guild_asset_url(flush_core::GuildAsset::Splash, sf, "h", Some(100)).unwrap(),
        "https://fluxerusercontent.com/splashes/42/h.png?size=480"
    );
    assert_eq!(flush_core::banner_url(sf, "", None), None);
}
