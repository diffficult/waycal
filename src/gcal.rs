use chrono::{DateTime, Datelike, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, BufRead, Write as IoWrite};
use std::path::PathBuf;

use crate::config::Config;

// ── Paths ─────────────────────────────────────────────────────────────────────

fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".config"));
    base.join("waycal")
}

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache"));
    base.join("waycal")
}

fn credentials_path() -> PathBuf {
    config_dir().join("credentials.json")
}

fn token_path() -> PathBuf {
    cache_dir().join("token.json")
}

fn events_cache_path(year: i32, month: u32) -> PathBuf {
    cache_dir().join(format!("events_{:04}-{:02}.json", year, month))
}

// ── Credentials file format (Google client_secret JSON) ──────────────────────

#[derive(Deserialize)]
struct CredentialsFile {
    installed: Option<InstalledCreds>,
    web: Option<InstalledCreds>,
}

#[derive(Deserialize)]
struct InstalledCreds {
    client_id: String,
    client_secret: String,
    auth_uri: String,
    token_uri: String,
}

impl CredentialsFile {
    fn inner(&self) -> Option<&InstalledCreds> {
        self.installed.as_ref().or(self.web.as_ref())
    }
}

// ── Stored token ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct StoredToken {
    access_token: String,
    refresh_token: Option<String>,
    /// RFC3339 timestamp when access_token expires
    expires_at: String,
}

impl StoredToken {
    fn is_valid(&self) -> bool {
        if let Ok(dt) = DateTime::parse_from_rfc3339(&self.expires_at) {
            let margin = chrono::Duration::seconds(60);
            dt.with_timezone(&Local) > Local::now() + margin
        } else {
            false
        }
    }
}

fn load_token() -> Option<StoredToken> {
    let path = token_path();
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_token(token: &StoredToken) {
    let path = token_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(token).unwrap();
    let tmp = path.with_extension("tmp");
    let _ = std::fs::write(&tmp, json);
    let _ = std::fs::rename(tmp, path);
}

// ── Token response from Google ────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

impl TokenResponse {
    fn into_stored(self, existing_refresh: Option<String>) -> StoredToken {
        let expires_in = self.expires_in.unwrap_or(3600);
        let expires_at = Local::now() + chrono::Duration::seconds(expires_in as i64);
        StoredToken {
            access_token: self.access_token,
            refresh_token: self.refresh_token.or(existing_refresh),
            expires_at: expires_at.to_rfc3339(),
        }
    }
}

// ── OAuth2 ────────────────────────────────────────────────────────────────────

const SCOPES: &str = "https://www.googleapis.com/auth/calendar.readonly";

pub fn get_access_token() -> Result<String, Box<dyn std::error::Error>> {
    // Fast path: valid cached token
    if let Some(tok) = load_token() {
        if tok.is_valid() {
            return Ok(tok.access_token);
        }
        // Try refresh
        if let Some(ref refresh_token) = tok.refresh_token {
            if let Ok(new_tok) = refresh_access_token(refresh_token) {
                save_token(&new_tok);
                return Ok(new_tok.access_token);
            }
        }
    }

    // Full auth flow
    let creds_text = std::fs::read_to_string(credentials_path())
        .map_err(|_| "credentials.json not found — copy your Google client_secret JSON to ~/.config/waycal/credentials.json")?;
    let creds_file: CredentialsFile = serde_json::from_str(&creds_text)?;
    let creds = creds_file.inner().ok_or("Invalid credentials format")?;

    let redirect_uri = "http://127.0.0.1";
    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        creds.auth_uri, creds.client_id, urlencoded(redirect_uri), SCOPES
    );

    eprintln!("\nOpen this URL in your browser to authorize waycal:\n\n{}\n", auth_url);
    eprint!("Paste the full redirect URL from your browser here: ");
    io::stderr().flush()?;

    let stdin = io::stdin();
    let pasted = stdin.lock().lines().next()
        .ok_or("No input")?
        .map_err(|e| e.to_string())?;
    let pasted = pasted.trim();

    // Extract code= from the redirect URL or treat the whole thing as a bare code
    let code = if let Some(qs) = pasted.split('?').nth(1) {
        qs.split('&').find_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            if kv.next() == Some("code") { kv.next().map(|v| v.to_string()) } else { None }
        }).ok_or("No code= param found in pasted URL")?
    } else {
        pasted.to_string()
    };

    // Exchange code for tokens
    let resp: TokenResponse = ureq::post(&creds.token_uri)
        .send_form(&[
            ("client_id",     &creds.client_id),
            ("client_secret", &creds.client_secret),
            ("redirect_uri",  &redirect_uri),
            ("grant_type",    "authorization_code"),
            ("code",          &code),
        ])?
        .into_json()?;

    let stored = resp.into_stored(None);
    save_token(&stored);
    Ok(stored.access_token)
}

fn refresh_access_token(refresh_token: &str) -> Result<StoredToken, Box<dyn std::error::Error>> {
    let creds_text = std::fs::read_to_string(credentials_path())?;
    let creds_file: CredentialsFile = serde_json::from_str(&creds_text)?;
    let creds = creds_file.inner().ok_or("Invalid credentials")?;

    let resp: TokenResponse = ureq::post(&creds.token_uri)
        .send_form(&[
            ("client_id",     &creds.client_id),
            ("client_secret", &creds.client_secret),
            ("grant_type",    "refresh_token"),
            ("refresh_token", refresh_token),
        ])?
        .into_json()?;

    Ok(resp.into_stored(Some(refresh_token.to_string())))
}

// ── Calendar events ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CalEvent {
    pub date: String,       // "YYYY-MM-DD"
    pub start_time: String, // "HH:MM" or "All day"
    pub end_time: String,
    pub title: String,
    pub calendar: String,
    pub color: String,
    pub icon: String,
    pub all_day: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MonthCache {
    pub fetched_at: String,
    pub month: String,
    pub events: Vec<CalEvent>,
}

impl MonthCache {
    pub fn events_for_date(&self, date: &str) -> Vec<&CalEvent> {
        self.events.iter().filter(|e| e.date == date).collect()
    }

    pub fn is_fresh(&self) -> bool {
        if let Ok(dt) = DateTime::parse_from_rfc3339(&self.fetched_at) {
            let age = Local::now().signed_duration_since(dt.with_timezone(&Local));
            age.num_hours() < 24
        } else {
            false
        }
    }
}

// ── Fetch ─────────────────────────────────────────────────────────────────────

pub fn load_or_fetch(year: i32, month: u32, config: &Config) -> Result<MonthCache, Box<dyn std::error::Error>> {
    let path = events_cache_path(year, month);
    if path.exists() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(cache) = serde_json::from_str::<MonthCache>(&text) {
                if cache.is_fresh() {
                    return Ok(cache);
                }
            }
        }
    }
    fetch_month(year, month, config)
}

fn fetch_month(year: i32, month: u32, config: &Config) -> Result<MonthCache, Box<dyn std::error::Error>> {
    let token = get_access_token()?;
    let auth = format!("Bearer {}", token);

    // Calendar list
    let cal_list_resp: serde_json::Value = ureq::get(
        "https://www.googleapis.com/calendar/v3/users/me/calendarList"
    )
    .set("Authorization", &auth)
    .call()?
    .into_json()?;

    let calendars = cal_list_resp["items"].as_array()
        .ok_or("No calendar items")?;

    // Date range for the month
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let time_min = format!("{:04}-{:02}-01T00:00:00Z", year, month);
    let time_max = format!("{:04}-{:02}-01T00:00:00Z", next_year, next_month);

    let mut all_events: Vec<CalEvent> = Vec::new();

    for cal in calendars {
        let cal_id = match cal["id"].as_str() { Some(s) => s, None => continue };
        let cal_name = cal["summary"].as_str().unwrap_or("Unknown");

        let events_resp: serde_json::Value = ureq::get(&format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events",
            urlencoded(cal_id)
        ))
        .set("Authorization", &auth)
        .query("timeMin", &time_min)
        .query("timeMax", &time_max)
        .query("singleEvents", "true")
        .query("orderBy", "startTime")
        .query("maxResults", "250")
        .call()?
        .into_json()?;

        let items = match events_resp["items"].as_array() {
            Some(a) => a,
            None => continue,
        };

        for item in items {
            let title = item["summary"].as_str().unwrap_or("(No title)").to_string();
            let start = &item["start"];
            let end   = &item["end"];

            let (date, start_time, all_day) = if let Some(dt_str) = start["dateTime"].as_str() {
                let dt = DateTime::parse_from_rfc3339(dt_str)?;
                let local = dt.with_timezone(&Local);
                (local.format("%Y-%m-%d").to_string(), local.format("%H:%M").to_string(), false)
            } else if let Some(d) = start["date"].as_str() {
                (d[..10].to_string(), "All day".to_string(), true)
            } else {
                continue;
            };

            let end_time = if !all_day {
                if let Some(dt_str) = end["dateTime"].as_str() {
                    let dt = DateTime::parse_from_rfc3339(dt_str)?;
                    dt.with_timezone(&Local).format("%H:%M").to_string()
                } else { String::new() }
            } else { String::new() };

            let cal_entry = config.calendars.iter().find(|e| e.name == cal_name);
            let color = cal_entry.map(|e| e.color.as_str()).unwrap_or(&config.default_cal.color);
            let icon  = cal_entry.map(|e| e.icon.as_str()).unwrap_or(&config.default_cal.icon);
            all_events.push(CalEvent {
                date,
                start_time,
                end_time,
                title,
                calendar: cal_name.to_string(),
                color: color.to_string(),
                icon:  icon.to_string(),
                all_day,
            });
        }
    }

    // Sort by date then start_time
    all_events.sort_by(|a, b| a.date.cmp(&b.date).then(a.start_time.cmp(&b.start_time)));

    let cache = MonthCache {
        fetched_at: Local::now().to_rfc3339(),
        month: format!("{:04}-{:02}", year, month),
        events: all_events,
    };

    // Write cache atomically
    let path = events_cache_path(year, month);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string(&cache)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)?;

    Ok(cache)
}

// ── Bar output ────────────────────────────────────────────────────────────────

pub fn bar_output(config: &Config) {
    let today = Local::now();
    let year = today.year();
    let month = today.month();
    let today_str = today.format("%Y-%m-%d").to_string();

    let path = events_cache_path(year, month);
    let (events_today, stale) = if path.exists() {
        match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<MonthCache>(&t).ok()) {
            Some(cache) => {
                let evs: Vec<_> = cache.events.iter()
                    .filter(|e| e.date == today_str)
                    .cloned()
                    .collect();
                let stale = !cache.is_fresh();
                (evs, stale)
            }
            None => (vec![], true),
        }
    } else {
        (vec![], true)
    };

    let count = events_today.len();
    let cal_icon = "\u{f073}"; //

    let output: HashMap<&str, serde_json::Value> = if stale && count == 0 {
        HashMap::from([
            ("text",    serde_json::json!(format!("<span size='14pt'>{} ⚠</span>", cal_icon))),
            ("tooltip", serde_json::json!("Calendar data stale or unavailable")),
            ("class",   serde_json::json!("calendar-stale")),
            ("alt",     serde_json::json!("0")),
        ])
    } else if count == 0 {
        HashMap::from([
            ("text",    serde_json::json!(format!("<span size='14pt'>{} </span>", cal_icon))),
            ("tooltip", serde_json::json!("No events today")),
            ("class",   serde_json::json!("calendar-empty")),
            ("alt",     serde_json::json!("0")),
        ])
    } else {
        let stale_badge = if stale { " ⚠" } else { "" };
        let text = format!(
            "<span size='14pt'>{} <span color='{}'>{}{}</span></span>",
            cal_icon, config.theme.bar_count_color, count, stale_badge
        );
        let tooltip = events_today.iter()
            .map(|e| format!("{}  {}", e.start_time, e.title))
            .collect::<Vec<_>>()
            .join("\n");
        HashMap::from([
            ("text",    serde_json::json!(text)),
            ("tooltip", serde_json::json!(tooltip)),
            ("class",   serde_json::json!("calendar-active")),
            ("alt",     serde_json::json!(count.to_string())),
        ])
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn urlencoded(s: &str) -> String {
    s.chars().flat_map(|c| {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '@') {
            vec![c]
        } else {
            // percent-encode
            c.to_string().as_bytes().iter()
                .flat_map(|b| format!("%{:02X}", b).chars().collect::<Vec<_>>())
                .collect()
        }
    }).collect()
}
