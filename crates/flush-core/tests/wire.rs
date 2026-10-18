use flush_core::events::DispatchEnvelope;
use flush_core::{Channel, ChannelType, Event, Guild, Message, MessageType, Role};

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
    v["type"] = serde_json::json!(99);
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
        other => panic!("esperava Ready, veio {}", other.kind()),
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
    // unicode → percent bytes; custom → nome:id com `:` escapado.
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
