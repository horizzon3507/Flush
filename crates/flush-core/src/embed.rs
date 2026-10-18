use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Embed de envio (subset `rich`): title/description/color/author/footer/image/thumbnail/fields.
/// Limites espelham o `EmbedBuilder` do `@fluxerjs/builders`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Embed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<EmbedAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<EmbedFooter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<EmbedMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<EmbedMedia>,
    #[serde(
        default,
        deserialize_with = "crate::wire::null_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub fields: Vec<EmbedField>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedAuthor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedFooter {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedMedia {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

const TITLE_MAX: usize = 256;
const DESCRIPTION_MAX: usize = 4096;
const FIELD_NAME_MAX: usize = 256;
const FIELD_VALUE_MAX: usize = 1024;
const FIELDS_MAX: usize = 25;
const TOTAL_MAX: usize = 6000;

#[derive(Debug, Clone, Default)]
pub struct EmbedBuilder {
    inner: Embed,
}

impl EmbedBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.inner.title = Some(title.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.inner.description = Some(description.into());
        self
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.inner.url = Some(url.into());
        self
    }

    pub fn color(mut self, color: u32) -> Self {
        self.inner.color = Some(color & 0xFF_FFFF);
        self
    }

    pub fn timestamp_now(mut self) -> Self {
        self.inner.timestamp = Some(now_rfc3339());
        self
    }

    pub fn author(mut self, name: impl Into<String>) -> Self {
        self.inner.author = Some(EmbedAuthor {
            name: name.into(),
            url: None,
            icon_url: None,
        });
        self
    }

    pub fn footer(mut self, text: impl Into<String>) -> Self {
        self.inner.footer = Some(EmbedFooter {
            text: text.into(),
            icon_url: None,
        });
        self
    }

    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.inner.image = Some(EmbedMedia {
            url: url.into(),
            description: None,
        });
        self
    }

    pub fn thumbnail(mut self, url: impl Into<String>) -> Self {
        self.inner.thumbnail = Some(EmbedMedia {
            url: url.into(),
            description: None,
        });
        self
    }

    pub fn field(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
        inline: bool,
    ) -> Self {
        self.inner.fields.push(EmbedField {
            name: name.into(),
            value: value.into(),
            inline,
        });
        self
    }

    pub fn build(self) -> Result<Embed, CoreError> {
        self.inner.validate()?;
        Ok(self.inner)
    }

    /// Versão tolerante: trunca em vez de errar (contagem por chars).
    pub fn build_lossy(mut self) -> Embed {
        truncate_opt(&mut self.inner.title, TITLE_MAX);
        truncate_opt(&mut self.inner.description, DESCRIPTION_MAX);
        if self.inner.fields.len() > FIELDS_MAX {
            self.inner.fields.truncate(FIELDS_MAX);
        }
        for f in &mut self.inner.fields {
            truncate_str(&mut f.name, FIELD_NAME_MAX);
            truncate_str(&mut f.value, FIELD_VALUE_MAX);
        }
        // Enforça teto total cortando a descrição.
        while char_count(&self.inner) > TOTAL_MAX {
            let Some(desc) = self.inner.description.as_mut() else {
                break;
            };
            if desc.is_empty() {
                break;
            }
            // Corta ~10% por iteração (em boundary de char).
            let cut = (desc.chars().count() / 10).max(1);
            let keep: String = desc.chars().take(desc.chars().count() - cut).collect();
            *desc = keep;
        }
        self.inner
    }
}

fn truncate_opt(v: &mut Option<String>, max_chars: usize) {
    if let Some(s) = v {
        truncate_str(s, max_chars);
    }
}

fn truncate_str(s: &mut String, max_chars: usize) {
    if s.chars().count() > max_chars {
        *s = s.chars().take(max_chars).collect();
    }
}

fn char_count(embed: &Embed) -> usize {
    let mut n = 0;
    n += embed.title.as_ref().map(|s| s.chars().count()).unwrap_or(0);
    n += embed
        .description
        .as_ref()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    for f in &embed.fields {
        n += f.name.chars().count() + f.value.chars().count();
    }
    if let Some(a) = &embed.author {
        n += a.name.chars().count();
    }
    if let Some(f) = &embed.footer {
        n += f.text.chars().count();
    }
    n
}

impl Embed {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.fields.len() > FIELDS_MAX {
            return Err(CoreError::InvalidEmbed(format!(
                "fields: {} > {FIELDS_MAX}",
                self.fields.len()
            )));
        }
        if self.title.as_ref().map(|s| s.chars().count()).unwrap_or(0) > TITLE_MAX {
            return Err(CoreError::InvalidEmbed("title > 256 chars".into()));
        }
        if self
            .description
            .as_ref()
            .map(|s| s.chars().count())
            .unwrap_or(0)
            > DESCRIPTION_MAX
        {
            return Err(CoreError::InvalidEmbed("description > 4096 chars".into()));
        }
        for f in &self.fields {
            if f.name.chars().count() > FIELD_NAME_MAX {
                return Err(CoreError::InvalidEmbed("field.name > 256 chars".into()));
            }
            if f.value.chars().count() > FIELD_VALUE_MAX {
                return Err(CoreError::InvalidEmbed("field.value > 1024 chars".into()));
            }
        }
        if char_count(self) > TOTAL_MAX {
            return Err(CoreError::InvalidEmbed("embed > 6000 chars".into()));
        }
        Ok(())
    }
}

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // ISO-8601 UTC sem dependência de chrono (algoritmo de Howard Hinnant).
    let days = (secs / 86_400) as i64;
    let secs_of_day = (secs % 86_400) as u32;
    let (y, m, d) = civil_from_days(days + 719_468);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
