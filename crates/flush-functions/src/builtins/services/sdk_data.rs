//! Embedded `@fluxerjs` SDK tables used by the `pure-*` builtins
//! (`sdkConstants`, `flagTables`, emoji shortcodes).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

static RAW_CONSTANTS: &str = include_str!("sdk_constants.json");
static RAW_SHORTCODES: &str = include_str!("emoji_shortcodes.json");

/// The `constants` object dumped from `@fluxerjs/core` + `@fluxerjs/util`.
pub static CONSTANTS: std::sync::LazyLock<Value> =
    std::sync::LazyLock::new(|| serde_json::from_str(RAW_CONSTANTS).expect("sdk constants"));

/// `flagTables`: lowercased table name → flag name → bit value (u128).
/// Mirrors `pure-flags.ts`: numeric entries only, `bits` stays empty.
pub fn flag_tables() -> &'static HashMap<String, HashMap<String, u128>> {
    static TABLES: OnceLock<HashMap<String, HashMap<String, u128>>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let names = [
            "permissions",
            "message",
            "user",
            "publicUser",
            "systemChannel",
            "threadMember",
            "memberProfile",
            "attachment",
            "embedMedia",
            "gatewayIdentify",
        ];
        let mut tables = HashMap::new();
        for name in names {
            tables.insert(name.to_lowercase(), table_for(name));
        }
        tables.insert("bits".into(), HashMap::new());
        tables
    })
}

fn table_for(name: &str) -> HashMap<String, u128> {
    const KEY: &[(&str, &str)] = &[
        ("permissions", "PermissionFlags"),
        ("message", "MessageFlagsBits"),
        ("user", "UserFlagsBits"),
        ("publicUser", "PublicUserFlags"),
        ("systemChannel", "SystemChannelFlags"),
        ("threadMember", "ThreadMemberFlags"),
        ("memberProfile", "GuildMemberProfileFlags"),
        ("attachment", "MessageAttachmentFlags"),
        ("embedMedia", "EmbedMediaFlags"),
        ("gatewayIdentify", "GatewayIdentifyFlags"),
    ];
    let Some(constant) = KEY
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .and_then(|(_, k)| CONSTANTS.get(k))
    else {
        return HashMap::new();
    };
    constant
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(k, v)| {
                    // TS numeric enums serialize both directions; flag tables
                    // keep only name → number.
                    v.as_u64().map(|n| (k.clone(), n as u128))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `UNICODE_EMOJI_SHORTCODES`.
pub fn shortcodes() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        serde_json::from_str::<HashMap<String, String>>(RAW_SHORTCODES).expect("emoji shortcodes")
    })
}

/// `DEFAULT_INSTANCE_ENDPOINTS`.
pub fn instance_endpoints() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        CONSTANTS
            .get("DEFAULT_INSTANCE_ENDPOINTS")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// `DEFAULT_CACHE_LIMITS`.
pub fn cache_limits() -> &'static serde_json::Map<String, Value> {
    static MAP: OnceLock<serde_json::Map<String, Value>> = OnceLock::new();
    MAP.get_or_init(|| {
        CONSTANTS
            .get("DEFAULT_CACHE_LIMITS")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    })
}
