mod config;
mod feeds;
mod ics;
mod model;
mod ui;

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::{Html, Response},
    routing::get,
};
use chrono::{DateTime, NaiveDate, Offset, Utc};
use clap::{Parser, Subcommand};
use config::Config;
use model::{Event, View, fetch_range};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

#[derive(Parser)]
#[command(version, about = "A lightweight ICS calendar kiosk")]
struct Args {
    #[arg(short, long, global = true, default_value = "calendar.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the calendar web server (the default).
    Serve,
}

#[derive(Default)]
struct Snapshot {
    events: Vec<Event>,
    updated_at: Option<DateTime<Utc>>,
    error: Option<String>,
}

struct App {
    config: Config,
    snapshot: RwLock<Snapshot>,
}

impl Snapshot {
    fn view(&self, config: &Config) -> View<'_> {
        let now = Utc::now().with_timezone(&config.timezone);
        View {
            now,
            show_legend: config.show_legend,
            events: &self.events,
            updated_at: self.updated_at,
            error: self.error.clone(),
        }
    }

    fn apply(&mut self, result: Result<Vec<Event>>) {
        match result {
            Ok(events) => {
                self.events = events;
                self.updated_at = Some(Utc::now());
                self.error = None;
            }
            Err(error) => {
                eprintln!("Calendar feed sync failed: {error:#}");
                self.error =
                    Some("Check the server log for calendar feed or connection errors".into());
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::load(&args.config)?;
    match args.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(config).await,
    }
}

async fn serve(config: Config) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("listening on {}", config.bind))?;
    let app = Arc::new(App {
        config,
        snapshot: RwLock::new(Snapshot::default()),
    });
    let worker = tokio::spawn(synchronize(app.clone()));
    let router = Router::new()
        .route("/", get(index))
        .route("/api/view", get(view))
        .route(
            "/calendar.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../assets/calendar.css"),
                )
            }),
        )
        .route(
            "/calendar.js",
            get(|| async {
                (
                    [(
                        header::CONTENT_TYPE,
                        "application/javascript; charset=utf-8",
                    )],
                    include_str!("../assets/calendar.js"),
                )
            }),
        )
        .layer(middleware::from_fn(response_headers))
        .with_state(app);
    eprintln!("Calendar listening on http://{}", listener.local_addr()?);
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await;
    worker.abort();
    result.context("serving calendar")
}

async fn synchronize(app: Arc<App>) {
    let mut client = None;
    let mut attempted_range: Option<(NaiveDate, NaiveDate)> = None;
    let mut last_attempt = None;
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let range = fetch_range(Utc::now().with_timezone(&app.config.timezone).date_naive());
        if attempted_range == Some(range)
            && last_attempt
                .is_some_and(|last: Instant| last.elapsed().as_secs() < app.config.refresh_seconds)
        {
            continue;
        }
        attempted_range = Some(range);
        let result = async {
            if client.is_none() {
                client = Some(feeds::FeedClient::new(app.config.calendars.clone())?);
            }
            client
                .as_ref()
                .expect("initialized client")
                .fetch(range.0, range.1, app.config.timezone)
                .await
        }
        .await;
        last_attempt = Some(Instant::now());
        app.snapshot.write().await.apply(result);
    }
}

async fn index(State(app): State<Arc<App>>) -> Html<String> {
    let snapshot = app.snapshot.read().await;
    Html(ui::page(&snapshot.view(&app.config)))
}

#[derive(Serialize)]
struct ViewResponse {
    html: String,
    upcoming_html: String,
    date: String,
    clock: String,
    now_ms: i64,
    offset_seconds: i32,
    status: String,
    updated_at_ms: Option<i64>,
}

async fn view(State(app): State<Arc<App>>) -> Json<ViewResponse> {
    let snapshot = app.snapshot.read().await;
    let view = snapshot.view(&app.config);
    Json(ViewResponse {
        html: ui::calendar(&view),
        upcoming_html: ui::upcoming(&view),
        date: ui::date(&view),
        clock: ui::clock(&view),
        now_ms: view.now.timestamp_millis(),
        offset_seconds: view.now.offset().fix().local_minus_utc(),
        status: ui::status(&view),
        updated_at_ms: view.updated_at.map(|updated| updated.timestamp_millis()),
    })
}

async fn response_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"));
    response
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("installing termination handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventTime;

    #[test]
    fn failed_sync_preserves_events_until_a_successful_replacement() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let mut snapshot = Snapshot::default();
        snapshot.apply(Ok(vec![Event {
            title: "Planning meeting".into(),
            calendar: "Team".into(),
            start: EventTime::AllDay(day),
            end: EventTime::AllDay(day.succ_opt().unwrap()),
            color: "#123456".into(),
        }]));
        let updated = snapshot.updated_at;
        snapshot.apply(Err(anyhow::anyhow!("offline")));
        assert_eq!(snapshot.events[0].title, "Planning meeting");
        assert_eq!(snapshot.updated_at, updated);
        assert!(snapshot.error.is_some());
        snapshot.apply(Ok(Vec::new()));
        assert!(snapshot.events.is_empty());
        assert!(snapshot.error.is_none());
    }
}
