use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::wire::null_bool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PresenceStatus {
    #[default]
    Online,
    Idle,
    Dnd,
    Invisible,
    Offline,
}

impl Serialize for PresenceStatus {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v = match self {
            Self::Online => "online",
            Self::Idle => "idle",
            Self::Dnd => "dnd",
            Self::Invisible => "invisible",
            Self::Offline => "offline",
        };
        s.serialize_str(v)
    }
}

impl<'de> Deserialize<'de> for PresenceStatus {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.to_ascii_lowercase().as_str() {
            "idle" => Self::Idle,
            "dnd" => Self::Dnd,
            "invisible" | "offline" => Self::Invisible,
            "unknown" | "" => Self::Online,
            _ => Self::Online,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub name: String,
    /// O wire usa `type`; renomeado para `kind` em Rust.
    #[serde(default, rename = "type", deserialize_with = "crate::wire::null_i32")]
    pub kind: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Activity {
    /// 4 = Custom (padrão usado pelo Sour para presença rotativa).
    pub fn custom(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: 4,
            url: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    #[serde(default)]
    pub status: PresenceStatus,
    #[serde(default, deserialize_with = "null_bool")]
    pub afk: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    #[serde(
        default,
        deserialize_with = "crate::wire::null_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub activities: Vec<Activity>,
}
