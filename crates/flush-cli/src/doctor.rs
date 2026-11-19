//! `flush doctor`: local checks only, never connects and never prints secrets.

use std::path::Path;
use std::process::Command;

use flush_framework::{find_config, read_env_file, ProjectConfig};
use serde::Serialize;

use crate::scaffold::RUST_REQUIREMENT;
use crate::VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub version: &'static str,
    pub ok: bool,
    pub offline: bool,
    pub checks: Vec<Check>,
}

fn check(name: &'static str, status: Status, message: impl Into<String>) -> Check {
    Check {
        name,
        status,
        message: message.into(),
    }
}

/// `(major, minor)` out of `rustc 1.85.0 (...)`.
pub fn parse_rustc_version(output: &str) -> Option<(u32, u32)> {
    let version = output.split_whitespace().nth(1)?;
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn rust_check() -> Check {
    let required = parse_rustc_version(&format!("rustc {RUST_REQUIREMENT}")).unwrap_or((1, 85));
    let output = Command::new("rustc").arg("--version").output();
    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            match parse_rustc_version(&text) {
                Some(found) if found >= required => check(
                    "rust",
                    Status::Pass,
                    format!("{text}; requires >={RUST_REQUIREMENT}."),
                ),
                _ => check(
                    "rust",
                    Status::Fail,
                    format!("{text}; requires >={RUST_REQUIREMENT}."),
                ),
            }
        }
        _ => check(
            "rust",
            Status::Fail,
            format!("rustc not found; install Rust >={RUST_REQUIREMENT}."),
        ),
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

/// Scheme, host and path of a URL, without credentials or query.
fn safe_url(url: &str) -> (String, String, String) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], rest[at..].trim_end_matches('/')),
        None => (rest, ""),
    };
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = if host_port.starts_with('[') {
        host_port
            .split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or("").to_owned()
    };
    (
        scheme.to_owned(),
        host,
        format!("{scheme}://{host_port}{path}"),
    )
}

fn client_check(config: &ProjectConfig) -> Check {
    let mut status = Status::Pass;
    let mut parts = Vec::new();
    let endpoints = [
        ("origin", &config.origin),
        ("api_base", &config.api_base),
        ("gateway_url", &config.gateway_url),
    ];
    for (field, value) in endpoints {
        let Some(value) = value else { continue };
        let (scheme, host, shown) = safe_url(value);
        if matches!(scheme.as_str(), "http" | "ws") && !is_loopback(&host) {
            status = Status::Warn;
            parts.push(format!("{field} without TLS ({shown})"));
        } else {
            parts.push(format!("{field} set ({shown})"));
        }
    }
    if parts.is_empty() {
        parts.push("default Fluxer endpoints through discovery".to_owned());
    }
    if config.api_base.is_some() != config.gateway_url.is_some() {
        parts.push("set both api_base and gateway_url to skip discovery".to_owned());
    }
    check("client", status, format!("{}.", parts.join("; ")))
}

fn storage_check(data: &Path) -> Check {
    let mut current = data.to_path_buf();
    loop {
        match std::fs::metadata(&current) {
            Ok(info) => {
                return if info.permissions().readonly() || (current == data && !info.is_dir()) {
                    check(
                        "storage",
                        Status::Fail,
                        "The data directory is not writable.",
                    )
                } else {
                    check(
                        "storage",
                        Status::Pass,
                        "The data directory or its closest existing ancestor is writable; nothing was written.",
                    )
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !current.pop() {
                    return check(
                        "storage",
                        Status::Fail,
                        "The data directory is unreachable.",
                    );
                }
            }
            Err(_) => {
                return check(
                    "storage",
                    Status::Fail,
                    "The data directory is unreachable.",
                )
            }
        }
    }
}

fn manifest_check(dir: &Path) -> Check {
    match std::fs::read_to_string(dir.join("Cargo.toml")) {
        Ok(text) if text.contains("flush") => check(
            "project",
            Status::Pass,
            "Cargo.toml found with a Flush dependency.",
        ),
        Ok(_) => check(
            "project",
            Status::Warn,
            "Cargo.toml has no Flush dependency.",
        ),
        Err(_) => check(
            "project",
            Status::Fail,
            "No Cargo.toml next to the config; run and dev need a Cargo project.",
        ),
    }
}

pub fn doctor(input: Option<&Path>) -> Report {
    let mut checks = vec![rust_check()];
    let finish = |checks: Vec<Check>| Report {
        version: VERSION,
        ok: checks.iter().all(|check| check.status != Status::Fail),
        offline: true,
        checks,
    };
    let path = match find_config(input) {
        Ok(path) => path,
        Err(_) => {
            checks.push(check("config", Status::Fail, "Config not found."));
            return finish(checks);
        }
    };
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut token = std::env::var("FLUXER_BOT_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty());
    if token.is_none() {
        match read_env_file(&dir) {
            Ok(pairs) => {
                token = pairs
                    .into_iter()
                    .find(|(key, value)| key == "FLUXER_BOT_TOKEN" && !value.trim().is_empty())
                    .map(|(_, value)| value);
            }
            Err(_) => checks.push(check("environment", Status::Warn, "Could not read .env.")),
        }
    }
    checks.push(if token.is_some() {
        check(
            "token",
            Status::Pass,
            "Token present; value hidden and authentication not tested.",
        )
    } else {
        check(
            "token",
            Status::Warn,
            "Token missing; only needed to run the bot.",
        )
    });
    match ProjectConfig::load(&path) {
        Ok(config) => {
            checks.push(check(
                "config",
                Status::Pass,
                format!("{} is valid; no login performed.", config.name),
            ));
            checks.push(client_check(&config));
            checks.push(storage_check(&config.data_path(&dir)));
        }
        Err(error) => checks.push(check(
            "config",
            Status::Fail,
            format!(
                "Invalid config ({}); use check for the diagnosis.",
                error.code
            ),
        )),
    }
    checks.push(manifest_check(&dir));
    finish(checks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rustc_versions() {
        assert_eq!(
            parse_rustc_version("rustc 1.97.1 (c980f4866 2026-06-30)"),
            Some((1, 97))
        );
        assert_eq!(parse_rustc_version("rustc 1.85"), Some((1, 85)));
        assert_eq!(parse_rustc_version("nonsense"), None);
    }

    #[test]
    fn client_check_warns_on_plain_http_and_hides_credentials() {
        let mut config = ProjectConfig::parse("name = \"demo\"\n").unwrap();
        assert_eq!(client_check(&config).status, Status::Pass);
        config.origin = Some("http://user:secret@example.com/api?key=1".into());
        let warned = client_check(&config);
        assert_eq!(warned.status, Status::Warn);
        assert!(!warned.message.contains("secret") && !warned.message.contains("key=1"));
        config.origin = Some("http://localhost:8080".into());
        assert_eq!(client_check(&config).status, Status::Pass);
    }

    #[test]
    fn reports_missing_config_and_never_prints_the_token() {
        let dir = std::env::temp_dir().join(format!("flush-doctor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = doctor(Some(&dir));
        assert!(!missing.ok);
        assert!(missing
            .checks
            .iter()
            .any(|c| c.name == "config" && c.status == Status::Fail));

        std::fs::write(dir.join("flush.toml"), "name = \"demo\"\n").unwrap();
        std::fs::write(dir.join(".env"), "FLUXER_BOT_TOKEN=super-secret-value\n").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[dependencies]\nflush = \"0\"\n").unwrap();
        let report = doctor(Some(&dir));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("super-secret-value"));
        assert!(report.offline);
        for name in ["config", "client", "storage", "project", "token"] {
            let found = report.checks.iter().find(|c| c.name == name).unwrap();
            if name != "token" || std::env::var_os("FLUXER_BOT_TOKEN").is_none() {
                assert_eq!(found.status, Status::Pass, "{name}: {}", found.message);
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
