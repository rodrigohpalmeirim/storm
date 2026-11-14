//! Recognizes torrent links (magnets and `.torrent` URLs) in clipboard text.

/// A link worth offering to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipLink {
    /// The link exactly as copied (trimmed).
    pub url: String,
    /// Human-readable name: the magnet's `dn`, the URL's file name, or a short hash.
    pub name: String,
    /// Lowercase hex v1 info hash, when the link carries one.
    pub info_hash: Option<String>,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Magnet,
    TorrentUrl,
}

/// Clipboards can hold anything; don't scan huge blobs.
const MAX_TEXT: usize = 64 * 1024;

/// The first magnet link or `.torrent` URL in `text` (whitespace-separated tokens).
pub fn find(text: &str) -> Option<ClipLink> {
    if text.len() > MAX_TEXT {
        return None;
    }
    text.split_whitespace().find_map(parse)
}

/// Parses one token as a magnet link or an http(s) URL to a `.torrent` file.
pub fn parse(token: &str) -> Option<ClipLink> {
    let token = token.trim().trim_matches(|c| matches!(c, '"' | '\'' | '<' | '>'));
    if let Some(query) = strip_prefix_ci(token, "magnet:?") {
        return parse_magnet(token, query);
    }
    if is_torrent_url(token) {
        let path = token.split(['?', '#']).next().unwrap_or(token);
        let file = path.rsplit('/').next().unwrap_or(path);
        let name = percent_decode(file, false);
        let name = name.strip_suffix(".torrent").or_else(|| name.strip_suffix(".TORRENT")).unwrap_or(&name).to_owned();
        return Some(ClipLink { url: token.to_owned(), name, info_hash: None, kind: LinkKind::TorrentUrl });
    }
    None
}

/// `http(s)://host/…/name.torrent`, ignoring any query string or fragment.
pub fn is_torrent_url(s: &str) -> bool {
    let Some(rest) = strip_prefix_ci(s, "https://").or_else(|| strip_prefix_ci(s, "http://")) else {
        return false;
    };
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    // Needs a host and a path segment.
    let Some((host, path)) = path.split_once('/') else { return false };
    !host.is_empty() && path.len() > ".torrent".len() && path.to_ascii_lowercase().ends_with(".torrent")
}

fn parse_magnet(url: &str, query: &str) -> Option<ClipLink> {
    let mut info_hash = None;
    let mut has_xt = false;
    let mut dn = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key.to_ascii_lowercase().as_str() {
            // Repeated xt params are allowed (`xt.1`, …); take the first v1 hash.
            k if k == "xt" || k.starts_with("xt.") => {
                has_xt = true;
                let value = percent_decode(value, false);
                if info_hash.is_none()
                    && let Some(hash) = strip_prefix_ci(&value, "urn:btih:")
                {
                    info_hash = normalize_hash(hash);
                }
            }
            "dn" if dn.is_none() => {
                let name = percent_decode(value, true).trim().to_owned();
                if !name.is_empty() {
                    dn = Some(name);
                }
            }
            _ => {}
        }
    }
    if !has_xt {
        return None;
    }
    let name = dn.or_else(|| info_hash.as_ref().map(|h| short_hash(h))).unwrap_or_else(|| "Unnamed torrent".to_owned());
    Some(ClipLink { url: url.to_owned(), name, info_hash, kind: LinkKind::Magnet })
}

/// Accepts 40-char hex or 32-char base32 and returns lowercase hex.
pub fn normalize_hash(hash: &str) -> Option<String> {
    match hash.len() {
        40 if hash.bytes().all(|b| b.is_ascii_hexdigit()) => Some(hash.to_ascii_lowercase()),
        32 => base32_to_hex(hash),
        _ => None,
    }
}

fn base32_to_hex(s: &str) -> Option<String> {
    let mut bits: u64 = 0;
    let mut nbits = 0;
    let mut out = String::with_capacity(40);
    for c in s.bytes() {
        let v = match c.to_ascii_uppercase() {
            c @ b'A'..=b'Z' => c - b'A',
            c @ b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | u64::from(v);
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push_str(&format!("{:02x}", (bits >> nbits) & 0xff));
        }
    }
    (out.len() == 40).then_some(out)
}

/// First 8 hex digits, the way clients usually abbreviate hashes.
pub fn short_hash(hex: &str) -> String {
    hex.chars().take(8).collect()
}

/// Decodes `%XX` escapes (and `+` as space in query values). Invalid escapes
/// are kept literally; invalid UTF-8 is replaced.
pub fn percent_decode(s: &str, plus_is_space: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() && hex_val(bytes[i + 1]).is_some() && hex_val(bytes[i + 2]).is_some() => {
                out.push(hex_val(bytes[i + 1]).unwrap_or(0) << 4 | hex_val(bytes[i + 2]).unwrap_or(0));
                i += 3;
                continue;
            }
            b'+' if plus_is_space => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix))
        .then(|| &s[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

    #[test]
    fn magnet_with_display_name() {
        let l = parse(&format!(
            "magnet:?xt=urn:btih:{}&dn=Ubuntu+24.04%20LTS%20%E2%9C%93&tr=udp%3A%2F%2Fx",
            HEX.to_uppercase()
        ))
        .unwrap();
        assert_eq!(l.kind, LinkKind::Magnet);
        assert_eq!(l.name, "Ubuntu 24.04 LTS \u{2713}");
        assert_eq!(l.info_hash.as_deref(), Some(HEX));
    }

    #[test]
    fn magnet_without_name_uses_short_hash() {
        let l = parse(&format!("magnet:?xt=urn:btih:{HEX}")).unwrap();
        assert_eq!(l.name, "c12fe1c0");
    }

    #[test]
    fn magnet_base32_hash() {
        // Base32 of the bytes 0x00..=0x13.
        let l = parse("magnet:?xt=urn:btih:AAAQEAYEAUDAOCAJBIFQYDIOB4IBCEQT&dn=x").unwrap();
        assert_eq!(l.info_hash.as_deref(), Some("000102030405060708090a0b0c0d0e0f10111213"));
        assert_eq!(
            normalize_hash("aaaqeayeaudaocajbifqydiob4ibceqt").as_deref(),
            Some("000102030405060708090a0b0c0d0e0f10111213")
        );
    }

    #[test]
    fn magnet_without_v1_hash_is_still_a_link() {
        let l = parse("magnet:?xt=urn:btmh:1220abcd&dn=v2").unwrap();
        assert_eq!(l.info_hash, None);
        assert_eq!(l.name, "v2");
        assert!(parse("magnet:?dn=no-hash").is_none());
        assert!(parse("magnet:?xt=urn:btih:nothex").is_some_and(|l| l.info_hash.is_none()));
    }

    #[test]
    fn torrent_urls() {
        assert!(is_torrent_url("https://example.org/files/a.torrent"));
        assert!(is_torrent_url("HTTP://example.org/A.TORRENT?token=1#x"));
        assert!(!is_torrent_url("https://example.org/a.torrent.html"));
        assert!(!is_torrent_url("https://example.org/"));
        assert!(!is_torrent_url("https://.torrent"));
        assert!(!is_torrent_url("ftp://example.org/a.torrent"));
        let l = parse("https://example.org/dl/My%20Show%20S01.torrent?k=v").unwrap();
        assert_eq!(l.kind, LinkKind::TorrentUrl);
        assert_eq!(l.name, "My Show S01");
    }

    #[test]
    fn find_in_text() {
        assert!(find("just some text https://example.org/page").is_none());
        let l = find(&format!("  see magnet:?xt=urn:btih:{HEX}&dn=a\n")).unwrap();
        assert_eq!(l.name, "a");
        assert_eq!(find("\"https://x.org/b.torrent\"").unwrap().url, "https://x.org/b.torrent");
    }

    #[test]
    fn percent_decoding_edge_cases() {
        assert_eq!(percent_decode("100%", false), "100%");
        assert_eq!(percent_decode("%zz%4", false), "%zz%4");
        assert_eq!(percent_decode("a+b%2B", true), "a b+");
        assert_eq!(percent_decode("a+b", false), "a+b");
    }
}
