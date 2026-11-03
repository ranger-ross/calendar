use crate::{config::FeedConfig, ics::parse_feed, model::Event};
use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use chrono_tz::Tz;
use reqwest::{Client, redirect::Policy};
use std::time::Duration;

const MAX_FEED_BYTES: usize = 10 * 1024 * 1024;

pub struct FeedClient {
    sources: Vec<FeedConfig>,
    http: Client,
}

impl FeedClient {
    pub fn new(sources: Vec<FeedConfig>) -> Result<Self> {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .referer(false)
            .redirect(Policy::custom(|attempt| {
                let url = attempt.url();
                if attempt.previous().len() >= 5 {
                    attempt.error("too many redirects")
                } else if !matches!(url.scheme(), "http" | "https")
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || (url.scheme() == "http"
                        && attempt.previous().iter().any(|url| url.scheme() == "https"))
                {
                    attempt.error("unsafe feed redirect")
                } else {
                    attempt.follow()
                }
            }))
            .user_agent(concat!("calendar-kiosk/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| anyhow::anyhow!("creating calendar feed HTTP client failed"))?;
        Ok(Self { sources, http })
    }

    pub async fn fetch(
        &self,
        start: NaiveDate,
        end: NaiveDate,
        timezone: Tz,
    ) -> Result<Vec<Event>> {
        if end <= start {
            bail!("calendar range end must be after its start (end is exclusive)");
        }
        let mut events = Vec::new();
        for (index, source) in self.sources.iter().enumerate() {
            // Never attach URLs, feed names, response bodies, or raw network errors.
            let fetched = self
                .fetch_source(source, start, end, timezone)
                .await
                .with_context(|| format!("calendar feed {}", index + 1))?;
            events.extend(fetched);
        }
        Ok(events)
    }

    async fn fetch_source(
        &self,
        source: &FeedConfig,
        start: NaiveDate,
        end: NaiveDate,
        timezone: Tz,
    ) -> Result<Vec<Event>> {
        let mut response = self
            .http
            .get(&source.url)
            .send()
            .await
            .map_err(network_error)?;
        if !response.status().is_success() {
            bail!("HTTP status {}", response.status().as_u16());
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_FEED_BYTES as u64)
        {
            bail!("feed exceeds the 10 MiB size limit");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            if chunk.len() > MAX_FEED_BYTES - bytes.len() {
                bail!("feed exceeds the 10 MiB size limit");
            }
            bytes.extend_from_slice(&chunk);
        }
        let text =
            String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("feed is not valid UTF-8"))?;
        let source = source.clone();
        tokio::task::spawn_blocking(move || parse_feed(&text, &source, start, end, timezone))
            .await
            .map_err(|_| anyhow::anyhow!("calendar feed parser failed"))?
            .context("invalid ICS calendar data")
    }
}

fn network_error(error: reqwest::Error) -> anyhow::Error {
    // Even errors without their URL can retain a secret redirect URL in a cause.
    let message = if error.is_timeout() {
        "calendar feed request timed out"
    } else if error.is_redirect() {
        "calendar feed redirect failed"
    } else if error.is_connect() {
        "calendar feed connection failed"
    } else if error.is_body() || error.is_decode() {
        "reading calendar feed response failed"
    } else {
        "calendar feed request failed"
    };
    anyhow::anyhow!(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::StatusCode, response::Redirect, routing::get};

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nX-WR-CALNAME:Public holidays\r\nBEGIN:VEVENT\r\nUID:holiday\r\nDTSTAMP:20260101T000000Z\r\nDTSTART;VALUE=DATE:20261103\r\nDTEND;VALUE=DATE:20261104\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    struct Server {
        url: String,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn server(router: Router) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Server { url, task }
    }

    fn source(url: String) -> FeedConfig {
        FeedConfig {
            url,
            color: "#123456".into(),
            name: Some("Team".into()),
        }
    }

    async fn fetch(client: &FeedClient) -> Result<Vec<Event>> {
        client
            .fetch(
                NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
                chrono_tz::UTC,
            )
            .await
    }

    #[tokio::test]
    async fn follows_redirect_and_parses_real_feed_with_configured_metadata() {
        let server = server(
            Router::new()
                .route(
                    "/private-secret",
                    get(|| async { Redirect::temporary("/feed") }),
                )
                .route("/feed", get(|| async { ICS })),
        )
        .await;
        let client =
            FeedClient::new(vec![source(format!("{}/private-secret", server.url))]).unwrap();
        let events = fetch(&client).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "Holiday");
        assert_eq!(events[0].calendar, "Team");
        assert_eq!(events[0].color, "#123456");
    }

    #[tokio::test]
    async fn any_failed_feed_rejects_the_entire_refresh_without_exposing_urls() {
        let server = server(Router::new().route("/good", get(|| async { ICS })).route(
            "/private-secret",
            get(|| async { (StatusCode::FORBIDDEN, "private-secret") }),
        ))
        .await;
        let client = FeedClient::new(vec![
            source(format!("{}/good", server.url)),
            source(format!("{}/private-secret", server.url)),
        ])
        .unwrap();
        let error = fetch(&client).await.err().unwrap();
        assert!(format!("{error:#}").contains("calendar feed 2"));
        assert!(format!("{error:#}").contains("403"));
        assert!(!format!("{error:?}").contains("private-secret"));
    }

    #[tokio::test]
    async fn redirect_loops_are_bounded_and_redacted() {
        let server = server(Router::new().route(
            "/private-secret",
            get(|| async { Redirect::temporary("/private-secret") }),
        ))
        .await;
        let client =
            FeedClient::new(vec![source(format!("{}/private-secret", server.url))]).unwrap();
        let error = fetch(&client).await.err().unwrap();
        assert!(format!("{error:#}").contains("redirect"));
        assert!(!format!("{error:?}").contains("private-secret"));
    }

    #[tokio::test]
    async fn oversized_and_malformed_feeds_are_rejected() {
        let server = server(Router::new()
            .route("/large", get(|| async { vec![b'x'; MAX_FEED_BYTES + 1] }))
            .route("/malformed", get(|| async {
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:invalid-date\r\nDTSTART:private-secret\r\nDTEND:20261104T000000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            }))).await;
        let client = FeedClient::new(vec![source(format!("{}/large", server.url))]).unwrap();
        let error = fetch(&client).await.err().unwrap();
        assert!(format!("{error:#}").contains("size limit"));
        let client = FeedClient::new(vec![source(format!("{}/malformed", server.url))]).unwrap();
        let error = fetch(&client).await.err().unwrap();
        assert!(format!("{error:#}").contains("invalid ICS"));
        assert!(!format!("{error:?}").contains("private-secret"));
    }
}
