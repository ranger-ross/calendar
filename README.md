# Calendar

> [!WARNING]
> 
> This project is pretty much vibe coded and just meant for myself.
> The rest of this README is basically AI slop

A small, single-binary Rust web server for a read-only ICS calendar display. Configure calendar feed URLs and colors in a TOML file. Assets are embedded; there is no Node.js build, CDN, or separate web directory to deploy.

The display shows the current month in seven Sunday-first columns, with the date and a 12-hour clock above it. Seconds are superscript. The first day of each month includes its month name beside the day number. Dark is the default theme; the top-right button switches to light and remembers the choice in that browser. All-day events use filled color bars; timed events show a time range and a color accent. Multi-day events appear on each occupied date.

The relative last-synced time appears beside the theme button only when the last successful sync was more than five minutes ago. It hides again after a successful sync. Loading and update errors remain visible immediately in the same header status area.

The top-right upcoming-event panel shows up to three current or upcoming events as compact, static cards that share the full panel width. Titles wrap to two lines, with longer titles truncated. The soonest event is on the left, with later events extending to the right. Cards use their feed colors and the display timezone and cover 30 calendar days including today, even across month boundaries. Finished events are removed locally as they expire, and the panel requests the next events. All-day events expire at the end of their final date. The panel also refreshes with the calendar every minute and stacks below the clock on narrow screens.

The frontend uses server-rendered HTML, ES5 JavaScript, XMLHttpRequest, and local assets for compatibility with modern and older browsers.

## Build and configure

Install a current stable Rust toolchain, then build on the target machine:

```sh
cargo build --release
cp calendar.example.toml calendar.toml
```

Edit `calendar.toml` with the display timezone and calendar feeds. The example URLs below are placeholders and must be replaced with direct ICS feed URLs:

```toml
bind = "0.0.0.0:8080"
timezone = "UTC"
refresh_seconds = 300

[[calendars]]
url = "https://example.com/calendar.ics"
color = "#31a35e"
name = "Events"
```

Add another `[[calendars]]` block for each calendar. Each entry needs an HTTP or HTTPS ICS `url` and a `color` in `#RRGGBB` format. `name` is optional and otherwise comes from the feed's calendar name. At least one calendar is required.

TOML inline arrays work too:

```toml
calendars = [
  { url = "https://example.com/team.ics", color = "#039be5", name = "Team" },
  { url = "https://example.com/community.ics", color = "#e67c73", name = "Community" },
]
```

Place an inline `calendars` array at the top level, alongside `timezone`, rather than inside another TOML table. Use either the inline array or `[[calendars]]` blocks, not both.

The IANA `timezone` controls both the clock and event placement, independently of the browser's settings. `bind` defaults to `0.0.0.0:8080`; use `127.0.0.1:8080` for local-only access. `refresh_seconds` defaults to 300 and must be at least 30.

The legend is hidden by default. To enable it, add `show_legend = true` at the top level of `calendar.toml`, before any `[[calendars]]` blocks, and restart the server.

Each feed uses its configured color. Google ICS exports do not reliably include Google's per-event color overrides, so those overrides are not reproduced.

Build on the deployment host or cross-compile for its operating system and CPU architecture. Rustls handles HTTPS without an OpenSSL runtime dependency. Only the binary and configuration need to be deployed.

## Google Calendar feed URLs

### Private calendars

On a computer, open Google Calendar, then go to **Settings > your calendar > Integrate calendar > Secret address in iCal format**. Copy the complete URL into a `[[calendars]]` entry. Do not use the browser address, calendar embed URL, or a Google Calendar ID.

You do not need to make a private calendar public. Treat its secret ICS URL like a password: anyone with the URL can read the exported calendar. Keep it in the server's private configuration, not in source control or screenshots. If it is exposed, reset the secret address in Google Calendar and update the configuration. Some Workspace administrators disable secret addresses.

For shared calendars, obtain the feed URL from the calendar owner. Access to a shared calendar in Google's interface does not necessarily provide access to its secret ICS address.

```sh
chmod 600 calendar.toml
```

Google's [instructions for obtaining a read-only iCal URL](https://support.google.com/calendar/answer/37648) explain the available addresses and restrictions.

### Public calendars and holidays

Public calendars and holiday calendars can be added using their public ICS URLs. The server downloads feeds directly; there is no need to subscribe to them in a Google account first.

Feed providers control which events they export and how quickly changes appear. Refreshing the app more frequently cannot bypass upstream caching or export restrictions.

## Run

```sh
./target/release/calendar --config calendar.toml
```

Open `http://localhost:8080/` on the server, or `http://SERVER_ADDRESS:8080/` from another device. `serve` is also available as an explicit subcommand. Configuration changes require restarting the server.

The clock ticks every second without a network request. The page checks the server every minute and when it becomes visible again. The server downloads the feeds at the configured interval and displays events overlapping the month grid, including visible dates from adjacent months. The grid rolls over automatically at the start of a month. Busy dates remain scrollable rather than dropping events.

The footer shows the age of the last successful sync, such as "less than a minute ago" or "3 minutes ago". It updates locally as time passes, including while the server or a feed is unavailable.

If any feed cannot be loaded, the running server retains the last complete successful calendar in memory and displays a stale-data message. If the browser loses the server, it keeps its existing display and reports the connection failure. A server restart clears the in-memory calendar, so the feeds must be reachable to load events again. Synchronization errors are logged to stderr without the feed URLs.

Feeds must contain UTF-8 ICS data and be no larger than 10 MiB. Supported events include all-day and timed entries, IANA timezones, `RRULE`/`RDATE`/`EXDATE` recurrence, and moved or canceled individual occurrences. Floating times use the feed's `X-WR-TIMEZONE`, or the configured display timezone when absent.

Custom non-IANA timezone definitions, deprecated `EXRULE`, and `RECURRENCE-ID;RANGE=THISANDFUTURE` are rejected with an error rather than displayed incorrectly. Recurrence expansion is bounded to 65,535 candidates per rule, including historical occurrences, and 100,000 occurrences per series or events per feed. Very dense or long-running recurrence histories can exceed these limits; the previous successful calendar remains visible.

### Start on boot with systemd

Create a dedicated system account named `calendar`. Install the binary at `/usr/local/bin/calendar` and the configuration at `/etc/calendar/calendar.toml`, readable only by that account. Adjust the account and paths for the deployment. Create `/etc/systemd/system/calendar.service`:

```ini
[Unit]
Description=Calendar display
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
User=calendar
ExecStart=/usr/local/bin/calendar --config /etc/calendar/calendar.toml
Restart=on-failure
RestartSec=10
UMask=0077
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

Then enable it:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now calendar
journalctl -u calendar -f
```

## Network safety

The display has no login and serves HTTP for compatibility with older browsers. Anyone who can reach the configured address and port can read the displayed calendar. Use a trusted network, restrict access with a firewall, and do not expose the server directly to the public internet. Feed URLs remain on the server and are not sent to the browser. Prefer HTTPS feed URLs so private feed access and event data are encrypted in transit. The browser contacts only the display server. Keep the browser and host operating system updated.
