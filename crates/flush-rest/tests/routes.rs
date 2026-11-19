use flush_core::Snowflake;
use flush_rest::Routes;

fn sf(n: u64) -> Snowflake {
    Snowflake(n)
}

#[test]
fn message_routes_match_sdk() {
    assert_eq!(Routes::channel_messages(sf(10)), "/channels/10/messages");
    assert_eq!(
        Routes::channel_message(sf(10), sf(20)),
        "/channels/10/messages/20"
    );
    assert_eq!(
        Routes::channel_bulk_delete(sf(10)),
        "/channels/10/messages/bulk-delete"
    );
    assert_eq!(Routes::channel_typing(sf(10)), "/channels/10/typing");
}

#[test]
fn guild_member_ban_routes() {
    assert_eq!(Routes::guild_member(sf(1), sf(2)), "/guilds/1/members/2");
    assert_eq!(Routes::guild_member_me(sf(1)), "/guilds/1/members/@me");
    assert_eq!(
        Routes::guild_member_role(sf(1), sf(2), sf(3)),
        "/guilds/1/members/2/roles/3"
    );
    assert_eq!(Routes::guild_ban(sf(1), sf(2)), "/guilds/1/bans/2");
    assert_eq!(Routes::guild_bans(sf(1)), "/guilds/1/bans");
    assert_eq!(Routes::guild_roles(sf(1)), "/guilds/1/roles");
}

#[test]
fn discovery_and_self_routes() {
    assert_eq!(Routes::current_user(), "/users/@me");
    assert_eq!(Routes::user(sf(7)), "/users/7");
    assert_eq!(Routes::gateway_bot(), "/gateway/bot");
    assert_eq!(Routes::guild(sf(9)), "/guilds/9");
    assert_eq!(Routes::guild_channels(sf(9)), "/guilds/9/channels");
}

#[test]
fn new_resource_routes() {
    assert_eq!(Routes::channel_pins(sf(3)), "/channels/3/messages/pins");
    assert_eq!(Routes::channel_pin(sf(3), sf(4)), "/channels/3/pins/4");
    assert_eq!(
        Routes::channel_reactions(sf(3), sf(4)),
        "/channels/3/messages/4/reactions"
    );
    assert_eq!(
        Routes::channel_reaction_me(sf(3), sf(4), "%F0%9F%8E%89"),
        "/channels/3/messages/4/reactions/%F0%9F%8E%89/@me"
    );
    assert_eq!(
        Routes::channel_reaction_users(sf(3), sf(4), "star%3A77"),
        "/channels/3/messages/4/reactions/star%3A77/users"
    );
    assert_eq!(Routes::channel_invites(sf(3)), "/channels/3/invites");
    assert_eq!(Routes::guild_invites(sf(1)), "/guilds/1/invites");
    assert_eq!(Routes::invite("flush-abc"), "/invites/flush-abc");
    assert_eq!(Routes::invite("a b"), "/invites/a%20b");
    assert_eq!(Routes::guild_role(sf(1), sf(5)), "/guilds/1/roles/5");
    assert_eq!(Routes::dm_channels(), "/users/@me/channels");
}

#[test]
fn thread_and_tag_routes_match_sdk() {
    assert_eq!(Routes::channel_threads(sf(1)), "/channels/1/threads");
    assert_eq!(
        Routes::channel_message_threads(sf(1), sf(2)),
        "/channels/1/messages/2/threads"
    );
    assert_eq!(
        Routes::channel_threads_archived_private(sf(1)),
        "/channels/1/threads/archived/private"
    );
    assert_eq!(
        Routes::channel_thread_member_me(sf(1)),
        "/channels/1/thread-members/@me"
    );
    assert_eq!(Routes::channel_tag(sf(1), sf(2)), "/channels/1/tags/2");
    assert_eq!(
        Routes::guild_active_threads(sf(1)),
        "/guilds/1/threads/active"
    );
}

#[test]
fn webhook_expression_and_misc_routes_match_sdk() {
    assert_eq!(Routes::channel_webhooks(sf(1)), "/channels/1/webhooks");
    assert_eq!(Routes::webhook_execute(sf(1), "a/b"), "/webhooks/1/a%2Fb");
    assert_eq!(
        Routes::webhook_message(sf(1), "tok", sf(3)),
        "/webhooks/1/tok/messages/3"
    );
    assert_eq!(Routes::guild_emoji(sf(1), sf(2)), "/guilds/1/emojis/2");
    assert_eq!(
        Routes::guild_stickers_bulk(sf(1)),
        "/guilds/1/stickers/bulk"
    );
    assert_eq!(Routes::guild_audit_logs(sf(1)), "/guilds/1/audit-logs");
    assert_eq!(
        Routes::channel_permission(sf(1), sf(2)),
        "/channels/1/permissions/2"
    );
    assert_eq!(Routes::leave_guild(sf(1)), "/users/@me/guilds/1");
    assert_eq!(
        Routes::user_profile(sf(1), Some(sf(2))),
        "/users/1/profile?guild_id=2"
    );
    assert_eq!(Routes::applications_me(), "/applications/@me");
    assert_eq!(Routes::instance_discovery(), "/.well-known/fluxer");
}
