//! Parse de comando com prefixo (espelho de `Sour lib/prefixCommand.ts`).

/// Resultado de `parse_prefix_body`: nome em minúsculas + args brutos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub name: String,
    pub args: Vec<String>,
}

/// `parsePrefixBody(content, prefix, case_sensitive=false)`.
pub fn parse_prefix_body(
    content: &str,
    prefix: &str,
    case_sensitive: bool,
) -> Option<ParsedCommand> {
    let starts = if case_sensitive {
        content.starts_with(prefix)
    } else {
        content.to_lowercase().starts_with(&prefix.to_lowercase())
    };
    if !starts {
        return None;
    }
    let body = content[prefix.len()..].trim();
    if body.is_empty() {
        return None;
    }
    let mut parts = body.split_whitespace();
    let raw = parts.next()?;
    if raw.is_empty() {
        return None;
    }
    Some(ParsedCommand {
        name: raw.to_lowercase(),
        args: parts.map(str::to_owned).collect(),
    })
}

/// `findPrefixCommand`: nome ou alias (case-insensitive).
pub fn find_command<'a, C: CommandDef<'a>>(name: &str, commands: &'a [C]) -> Option<&'a C> {
    let lower = name.to_lowercase();
    commands
        .iter()
        .find(|c| c.name() == lower || c.aliases().iter().any(|a| a.to_lowercase() == lower))
}

pub trait CommandDef<'a> {
    fn name(&self) -> &str;
    fn aliases(&self) -> &[String];
}

/// Resolve alvo `<@123>` / `<@!123>` / id 17-20 dígitos (padrão Sour).
pub fn parse_mention_or_id(raw: &str) -> Option<String> {
    let t = raw.trim();
    if let Some(id) = t.strip_prefix("<@&") {
        // Menção de cargo: `<@&id>` — o chamador decide (darcargo exige este formato).
        return id.strip_suffix('>').map(str::to_owned);
    }
    if let Some(rest) = t.strip_prefix("<@") {
        let id = rest.strip_prefix('!').unwrap_or(rest).strip_suffix('>')?;
        return valid_id(id).then(|| id.to_owned());
    }
    if valid_id(t) {
        return Some(t.to_owned());
    }
    None
}

fn valid_id(s: &str) -> bool {
    (17..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// Parse de args estilo Jully: `"aspas"`, `'aspas'`, `--flag valor`/`--flag`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RichArgs {
    pub args: Vec<String>,
    pub flags: Vec<(String, Option<String>)>,
}

pub fn parse_rich_args(input: &str) -> RichArgs {
    let mut out = RichArgs::default();
    let mut cur = String::new();
    let mut quote = None;
    let mut tokens: Vec<String> = Vec::new();
    for ch in input.chars() {
        match quote {
            Some(q) if ch == q => {
                quote = None;
            }
            Some(_) => cur.push(ch),
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push(ch),
            },
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if let Some(flag) = t.strip_prefix("--") {
            let next = tokens.get(i + 1);
            match next {
                Some(n) if !n.starts_with("--") => {
                    out.flags.push((flag.to_owned(), Some(n.clone())));
                    i += 2;
                }
                _ => {
                    out.flags.push((flag.to_owned(), None));
                    i += 1;
                }
            }
        } else {
            out.args.push(t.clone());
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_case_insensitive() {
        let p = parse_prefix_body("!Ping x", "!", false).unwrap();
        assert_eq!(p.name, "ping");
        assert_eq!(p.args, ["x"]);
        assert!(parse_prefix_body("?ping", "!", false).is_none());
        assert!(parse_prefix_body("!", "!", false).is_none());
    }

    #[test]
    fn mentions_and_ids() {
        assert_eq!(
            parse_mention_or_id("<@123456789012345678>"),
            Some("123456789012345678".into())
        );
        assert_eq!(
            parse_mention_or_id("<@!123456789012345678>"),
            Some("123456789012345678".into())
        );
        assert_eq!(
            parse_mention_or_id("123456789012345678"),
            Some("123456789012345678".into())
        );
        assert_eq!(parse_mention_or_id("abc"), None);
    }

    #[test]
    fn rich_args_quotes_and_flags() {
        let r = parse_rich_args(r#"say "ola mundo" --tempo 5 --x"#);
        assert_eq!(r.args, ["say", "ola mundo"]);
        assert_eq!(
            r.flags,
            [
                ("tempo".to_owned(), Some("5".into())),
                ("x".to_owned(), None)
            ]
        );
    }
}
