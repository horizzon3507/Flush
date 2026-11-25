use flush::core::Snowflake;
use flush::framework::GuildStore;

/// Reason text from the arguments after the first `skip`, or a default.
pub fn reason_from(args: &[String], skip: usize) -> String {
    let reason = args
        .iter()
        .skip(skip)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if reason.is_empty() {
        "No reason given".to_owned()
    } else {
        reason
    }
}

/// Mod-log channel configured for the guild, if any.
pub fn mod_log_channel(guild: Snowflake) -> Option<Snowflake> {
    GuildStore::with_default_path()
        .get(&guild.to_string())
        .mod_log_channel_id
        .and_then(|id| id.parse().ok())
}

#[cfg(test)]
mod moderation_tests {
    use super::reason_from;

    #[test]
    fn joins_the_reason_or_falls_back() {
        let args = vec!["@user".to_owned(), "spam".to_owned(), "links".to_owned()];
        assert_eq!(reason_from(&args, 1), "spam links");
        assert_eq!(reason_from(&args, 3), "No reason given");
    }
}
