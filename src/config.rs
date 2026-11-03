use anyhow::{Context, Result, bail};
use chrono_tz::Tz;
use serde::Deserialize;
use std::{net::SocketAddr, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    pub timezone: Tz,
    #[serde(default = "default_refresh")]
    pub refresh_seconds: u64,
    #[serde(default)]
    pub show_legend: bool,
    pub calendars: Vec<FeedConfig>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedConfig {
    pub url: String,
    pub color: String,
    pub name: Option<String>,
}

fn default_bind() -> SocketAddr {
    "0.0.0.0:8080".parse().unwrap()
}
fn default_refresh() -> u64 {
    300
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self> {
        // TOML errors include source snippets, which can expose secret feed URLs.
        let config: Self = toml::from_str(text)
            .map_err(|_| anyhow::anyhow!("invalid configuration TOML or configuration fields"))?;
        if config.refresh_seconds < 30 {
            bail!("refresh_seconds must be at least 30");
        }
        if config.calendars.is_empty() {
            bail!("at least one calendar feed is required");
        }
        for (index, source) in config.calendars.iter().enumerate() {
            let url = reqwest::Url::parse(&source.url)
                .map_err(|_| anyhow::anyhow!("calendar {} has an invalid URL", index + 1))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                bail!(
                    "calendar {} requires an HTTP(S) URL without credentials",
                    index + 1
                );
            }
            let color = source.color.as_bytes();
            if color.len() != 7 || color[0] != b'#' || !color[1..].iter().all(u8::is_ascii_hexdigit)
            {
                bail!("calendar {} color must be #RRGGBB", index + 1);
            }
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(url: &str, color: &str) -> String {
        format!("timezone = \"UTC\"\n[[calendars]]\nurl = \"{url}\"\ncolor = \"{color}\"\n")
    }

    #[test]
    fn requires_feeds_fields_and_valid_colors() {
        for text in [
            "timezone = \"UTC\"\n".to_owned(),
            "timezone = \"UTC\"\ncalendars = []\n".to_owned(),
            "timezone = \"UTC\"\n[[calendars]]\nurl = \"https://example.com\"\n".to_owned(),
            "timezone = \"UTC\"\n[[calendars]]\ncolor = \"#123456\"\n".to_owned(),
            config("https://example.com", "red"),
            config("https://example.com", "#12345g"),
            config("https://example.com", "#12345678"),
        ] {
            assert!(Config::parse(&text).is_err());
        }
    }

    #[test]
    fn preserves_refresh_minimum_and_timezone_validation() {
        let text = config("https://example.com", "#123456");
        assert!(Config::parse(&format!("refresh_seconds = 29\n{text}")).is_err());
        assert!(Config::parse(&format!("refresh_seconds = 30\n{text}")).is_ok());
        assert!(Config::parse(&text.replace("\"UTC\"", "\"Not/AZone\"")).is_err());
    }

    #[test]
    fn invalid_urls_and_toml_never_expose_secrets() {
        for text in [
            config("file:///private-secret.ics", "#123456"),
            config(
                "https://user:private-secret@example.com/basic.ics",
                "#123456",
            ),
            config("private-secret", "#123456"),
            config("https://example.com/private-secret", "#123456")
                .replace("url =", "unknown_private-secret ="),
            "timezone = \"UTC\"\n[[calendars]]\nurl = \"https://example.com/private-secret\n"
                .to_owned(),
        ] {
            let error = match Config::parse(&text) {
                Ok(_) => panic!("invalid configuration was accepted"),
                Err(error) => error,
            };
            assert!(!format!("{error:#}").contains("private-secret"));
            assert!(!format!("{error:?}").contains("private-secret"));
        }
    }
}
