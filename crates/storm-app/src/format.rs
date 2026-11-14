//! Pure formatting/parsing helpers. Everything here is unit-tested.

use storm_proto::TorrentState;

const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

/// Human-readable size in binary units: `1.5 MiB`, `512 B`.
pub fn bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    // Keep three significant digits: 1.23 / 12.3 / 123.
    let s = if v < 10.0 {
        format!("{v:.2}")
    } else if v < 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    };
    format!("{s} {}", UNITS[unit])
}

/// Speed in binary units per second, `""` for zero (keeps tables quiet).
pub fn speed(bps: u64) -> String {
    if bps == 0 { String::new() } else { format!("{}/s", bytes(bps)) }
}

/// Like [`speed`] but renders zero as `0 B/s` (for the status bar).
pub fn speed_always(bps: u64) -> String {
    format!("{}/s", bytes(bps))
}

/// Compact duration: `45s`, `3m 05s`, `2h 07m`, `3d 4h`, `∞` beyond a year.
pub fn eta(secs: Option<u64>) -> String {
    let Some(s) = secs else { return String::new() };
    if s >= 365 * 86_400 {
        return "∞".into();
    }
    let (d, h, m, sec) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60, s % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {sec:02}s")
    } else {
        format!("{sec}s")
    }
}

/// `0.0..=1.0` → `42.3%`. Exactly-complete shows `100%`.
pub fn percent(fraction: f64) -> String {
    let p = (fraction * 100.0).clamp(0.0, 100.0);
    if p >= 100.0 {
        "100%".into()
    } else {
        // Never round 99.96 up to "100.0%" while incomplete.
        format!("{:.1}%", (p * 10.0).floor() / 10.0)
    }
}

pub fn ratio(r: f64) -> String {
    if !r.is_finite() { "∞".into() } else { format!("{r:.2}") }
}

pub fn state(s: TorrentState) -> &'static str {
    match s {
        TorrentState::FetchingMetadata => "Fetching metadata",
        TorrentState::Initializing => "Checking",
        TorrentState::Downloading => "Downloading",
        TorrentState::Seeding => "Seeding",
        TorrentState::Paused => "Paused",
        TorrentState::Error => "Error",
    }
}

/// Unix seconds → local `YYYY-MM-DD HH:MM`.
pub fn date(unix: Option<i64>) -> String {
    use chrono::TimeZone as _;
    unix.and_then(|t| chrono::Local.timestamp_opt(t, 0).single())
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Unix seconds → compact local date for tables: `Today 14:02`,
/// `Yesterday 09:15`, `Nov 3`, or `2024-11-03` for other years.
pub fn short_date(unix: Option<i64>) -> String {
    use chrono::TimeZone as _;
    unix.and_then(|t| chrono::Local.timestamp_opt(t, 0).single())
        .map(|dt| short_date_at(dt.naive_local(), chrono::Local::now().naive_local()))
        .unwrap_or_default()
}

fn short_date_at(dt: chrono::NaiveDateTime, now: chrono::NaiveDateTime) -> String {
    use chrono::Datelike as _;
    let days = (now.date() - dt.date()).num_days();
    if days == 0 {
        dt.format("Today %H:%M").to_string()
    } else if days == 1 {
        dt.format("Yesterday %H:%M").to_string()
    } else if dt.year() == now.year() {
        format!("{} {}", dt.format("%b"), dt.day())
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

/// Parses a rate limit typed in KiB/s. Empty (or 0) = unlimited.
/// Returns the value in bytes per second, as the API wants it.
pub fn parse_limit_kib(input: &str) -> Result<Option<u32>, String> {
    let t = input.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let kib: u64 = t.parse().map_err(|_| format!("“{t}” is not a whole number of KiB/s"))?;
    if kib == 0 {
        return Ok(None);
    }
    let bps = kib.checked_mul(1024).filter(|&b| b <= u32::MAX as u64).ok_or("limit is too large")?;
    Ok(Some(bps as u32))
}

/// Inverse of [`parse_limit_kib`] for pre-filling inputs.
pub fn limit_to_kib(bps: Option<u32>) -> String {
    match bps {
        None | Some(0) => String::new(),
        Some(b) => ((b as u64).div_ceil(1024)).to_string(),
    }
}

pub fn parse_port(input: &str) -> Result<u16, String> {
    let t = input.trim();
    t.parse::<u16>().map_err(|_| format!("“{t}” is not a valid port (0–65535)"))
}

/// Splits pasted text into magnet links / URLs (one per whitespace-separated token).
pub fn extract_links(text: &str) -> Vec<String> {
    text.split_whitespace().filter(|t| is_torrent_link(t)).map(str::to_owned).collect()
}

pub fn is_torrent_link(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    l.starts_with("magnet:?") || l.starts_with("http://") || l.starts_with("https://")
}

/// A shareable magnet link for an info hash, with the name as `dn` when known.
pub fn magnet_link(info_hash: &str, name: Option<&str>) -> String {
    let mut link = format!("magnet:?xt=urn:btih:{info_hash}");
    if let Some(name) = name.filter(|n| !n.is_empty()) {
        link.push_str("&dn=");
        for b in name.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => link.push(b as char),
                b' ' => link.push('+'),
                _ => link.push_str(&format!("%{b:02X}")),
            }
        }
    }
    link
}

/// Last path component of a `/`- or `\`-separated path.
/// The host (and non-default port) of a server URL, for display.
pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    authority.strip_suffix(&format!(":{}", storm_proto::DEFAULT_PORT)).unwrap_or(authority)
}

pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_binary_units() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.00 KiB");
        assert_eq!(bytes(1536), "1.50 KiB");
        assert_eq!(bytes(10 * 1024 * 1024), "10.0 MiB");
        assert_eq!(bytes(700 * 1024 * 1024), "700 MiB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.00 GiB");
        assert_eq!(bytes(u64::MAX), "16384 PiB");
    }

    #[test]
    fn speed_hides_zero() {
        assert_eq!(speed(0), "");
        assert_eq!(speed(2048), "2.00 KiB/s");
        assert_eq!(speed_always(0), "0 B/s");
    }

    #[test]
    fn eta_formats() {
        assert_eq!(eta(None), "");
        assert_eq!(eta(Some(0)), "0s");
        assert_eq!(eta(Some(45)), "45s");
        assert_eq!(eta(Some(185)), "3m 05s");
        assert_eq!(eta(Some(2 * 3600 + 7 * 60)), "2h 07m");
        assert_eq!(eta(Some(3 * 86_400 + 4 * 3600)), "3d 4h");
        assert_eq!(eta(Some(400 * 86_400)), "∞");
    }

    #[test]
    fn percent_never_shows_100_when_incomplete() {
        assert_eq!(percent(0.0), "0.0%");
        assert_eq!(percent(0.4237), "42.3%");
        assert_eq!(percent(0.99999), "99.9%");
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(1.5), "100%");
    }

    #[test]
    fn limits_roundtrip() {
        assert_eq!(parse_limit_kib(""), Ok(None));
        assert_eq!(parse_limit_kib("  "), Ok(None));
        assert_eq!(parse_limit_kib("0"), Ok(None));
        assert_eq!(parse_limit_kib("100"), Ok(Some(102_400)));
        assert!(parse_limit_kib("abc").is_err());
        assert!(parse_limit_kib("-5").is_err());
        assert!(parse_limit_kib("99999999999").is_err());
        assert_eq!(limit_to_kib(None), "");
        assert_eq!(limit_to_kib(Some(102_400)), "100");
        assert_eq!(limit_to_kib(Some(1000)), "1");
    }

    #[test]
    fn ports() {
        assert_eq!(parse_port(" 6881 "), Ok(6881));
        assert_eq!(parse_port("0"), Ok(0));
        assert!(parse_port("70000").is_err());
        assert!(parse_port("").is_err());
    }

    #[test]
    fn links_are_extracted() {
        let text = "magnet:?xt=urn:btih:abc\n  junk https://x.org/a.torrent\tftp://nope";
        assert_eq!(extract_links(text), vec!["magnet:?xt=urn:btih:abc", "https://x.org/a.torrent"]);
        assert!(extract_links("hello world").is_empty());
    }

    #[test]
    fn host_of_strips_scheme_path_and_default_port() {
        assert_eq!(host_of("http://nas.local:9870"), "nas.local");
        assert_eq!(host_of("https://user@box.example.com/storm/"), "box.example.com");
        assert_eq!(host_of("http://10.0.0.5:8080/api"), "10.0.0.5:8080");
        assert_eq!(host_of("seedbox"), "seedbox");
    }

    #[test]
    fn short_dates() {
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        let now = at("2026-11-11 10:00");
        assert_eq!(short_date_at(at("2026-11-11 08:05"), now), "Today 08:05");
        assert_eq!(short_date_at(at("2026-11-10 23:59"), now), "Yesterday 23:59");
        assert_eq!(short_date_at(at("2026-03-02 12:00"), now), "Mar 2");
        assert_eq!(short_date_at(at("2025-11-11 12:00"), now), "2025-11-11");
        assert_eq!(short_date(None), "");
    }

    #[test]
    fn magnet_links() {
        assert_eq!(magnet_link("abc", None), "magnet:?xt=urn:btih:abc");
        assert_eq!(magnet_link("abc", Some("")), "magnet:?xt=urn:btih:abc");
        assert_eq!(magnet_link("abc", Some("Big Buck & Bunny")), "magnet:?xt=urn:btih:abc&dn=Big+Buck+%26+Bunny");
        assert_eq!(magnet_link("abc", Some("é")), "magnet:?xt=urn:btih:abc&dn=%C3%A9");
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name("a/b/c.mkv"), "c.mkv");
        assert_eq!(file_name(r"C:\x\y.iso"), "y.iso");
        assert_eq!(file_name("dir/"), "dir");
        assert_eq!(file_name("plain"), "plain");
    }
}
