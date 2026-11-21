//! The personal-data rules of the URL normalization (rule set version 3): which query and path parameters are dropped before a
//! job or ad id is computed. The lists are DATA at the top of this file; the TypeScript twin (`url-rules.ts` in
//! `@cv/jobcache`) holds the same lists, and both are pinned to `tests/fixtures/normalize-url.json` (the fixture carries the
//! lists too, so a list edited on one side only fails the other side's test).
//!
//! A parameter is dropped when ANY of these holds, checked in this order:
//! 1. its percent-decoded value contains an email address, or base64-decodes (at least [`BASE64_MIN_LEN`] characters, valid
//!    base64, printable result) to text containing one. This applies even to allow-listed names;
//! 2. its name (case-insensitive, unless allow-listed) contains one of [`NAME_CONTAINS`], equals one of [`NAME_EQUALS`], ends
//!    with one of [`NAME_SUFFIXES`] or starts with one of [`NAME_PREFIXES`];
//! 3. it is a host-scoped alert parameter of [`HOST_PARAMS`] (never for allow-listed names).
//!
//! Separators: `&`, and `;` only where a `name=` token follows it ([`split_params`]), so a `;` inside a value or an entity such as
//! `&apos;` never splits. Any userinfo (`user:pass@host`) or an email-shaped path segment makes the URL personal as a whole
//! ([`is_personal_url`]); the path is never rewritten.

/// The version of the URL rule set. 1: tracking parameters, fragment, trailing slash, unreserved percent-escapes, sorted keys.
/// 2: a first, narrow personal-data rule (ccvl 0.1.12, superseded before any id was computed with it). 3: this module.
pub const URL_RULE_VERSION: u32 = 3;

/// Separators of query parameters (and of path parameters after a path segment). `;` separates only before a `name=` token.
pub const QUERY_SEPARATORS: &[char] = &['&', ';'];

/// Rule 2, name contains (lowercase).
pub const NAME_CONTAINS: &[&str] = &[
    "token",
    "secret",
    "passw",
    "session",
    "signature",
    "credential",
    "apikey",
    "api_key",
];
/// Rule 2, name equals (lowercase).
pub const NAME_EQUALS: &[&str] = &["auth", "authorization", "pass", "pwd", "passwd"];
/// Rule 2, name ends with (lowercase).
pub const NAME_SUFFIXES: &[&str] = &["sig", "hash"];
/// Rule 2, name starts with (lowercase).
pub const NAME_PREFIXES: &[&str] = &["otp"];
/// Names boards use as job ids: never dropped by rules 2 and 3 (lowercase). Rule 1 still applies to their values.
pub const ALLOW_NAMES: &[&str] = &[
    "jk",
    "vjk",
    "id",
    "jid",
    "key",
    "uid",
    "sid",
    "currentjobid",
    "jobid",
    "gh_jid",
    "joblistingid",
];
/// Rule 3: host-scoped alert parameters (lowercase names). A host pattern `domain.tld` matches the domain and its
/// subdomains; `label.*` matches any host with that label followed by at least one more label.
pub const HOST_PARAMS: &[(&str, &[&str])] = &[
    ("linkedin.com", &["eid", "refid", "trackingid"]),
    ("xing.com", &["lk"]),
    ("indeed.*", &["alid", "qd", "rd"]),
    ("stepstone.*", &["user_token", "emailhash"]),
    ("glassdoor.*", &["guid", "jrtk"]),
    ("jobs.ch", &["jobalert_id"]),
];
/// Rule 1: shortest value tried as base64.
pub const BASE64_MIN_LEN: usize = 16;

fn is_local_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-') || (c as u32) >= 0x80
}

fn is_domain_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-') || (c as u32) >= 0x80
}

/// Does `s` contain an email address? Around each `@`: a non-empty run of local characters before it, and after it a
/// domain run (trailing dots and hyphens trimmed) that does not start with `.`, has a `.` and ends in a top-level label of
/// at least two letters. Non-ASCII characters count as letters and as local/domain characters. Written without a regex so the
/// TypeScript twin matches exactly.
#[must_use]
pub fn contains_email(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c != '@' {
            continue;
        }
        if i == 0 || !is_local_char(chars[i - 1]) {
            continue;
        }
        let mut end = i + 1;
        while end < chars.len() && is_domain_char(chars[end]) {
            end += 1;
        }
        let mut dom = &chars[i + 1..end];
        while let Some(&last) = dom.last() {
            if last == '.' || last == '-' {
                dom = &dom[..dom.len() - 1];
            } else {
                break;
            }
        }
        if dom.first() == Some(&'.') {
            continue;
        }
        if let Some(dot) = dom.iter().rposition(|&c| c == '.') {
            let tld = &dom[dot + 1..];
            if dot >= 1
                && tld.len() >= 2
                && tld
                    .iter()
                    .all(|&c| c.is_ascii_alphabetic() || (c as u32) >= 0x80)
            {
                return true;
            }
        }
    }
    false
}

fn b64_value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Standard or URL-safe base64, padding optional. `None` for anything that is not valid base64.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let trimmed = s.trim_end_matches('=');
    if s.len() - trimmed.len() > 2 || trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &b in trimmed.as_bytes() {
        buffer = (buffer << 6) | b64_value(b)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
        buffer &= (1 << bits) - 1;
    }
    Some(out)
}

/// A value of at least [`BASE64_MIN_LEN`] characters that is valid base64 whose printable text contains an email address.
#[must_use]
pub fn base64_email(s: &str) -> bool {
    if s.chars().count() < BASE64_MIN_LEN {
        return false;
    }
    let Some(bytes) = base64_decode(s) else {
        return false;
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return false;
    };
    if text
        .chars()
        .any(|c| (c < ' ' && !matches!(c, '\t' | '\n' | '\r')) || c == '\u{7f}')
    {
        return false;
    }
    contains_email(&text)
}

fn host_matches(pattern: &str, host: &str) -> bool {
    if let Some(label) = pattern.strip_suffix(".*") {
        let labels: Vec<&str> = host.split('.').collect();
        return labels
            .iter()
            .enumerate()
            .any(|(i, l)| *l == label && i + 1 < labels.len());
    }
    host == pattern || host.ends_with(&format!(".{pattern}"))
}

/// Should the parameter `name=value` of a URL on `host` be dropped? See the module documentation for the order.
#[must_use]
pub fn drop_param(host: &str, name: &str, value: &str) -> bool {
    if contains_email(value) || base64_email(value) {
        return true;
    }
    let n = name.to_lowercase();
    if ALLOW_NAMES.contains(&n.as_str()) {
        return false;
    }
    if NAME_CONTAINS.iter().any(|w| n.contains(w))
        || NAME_EQUALS.contains(&n.as_str())
        || NAME_SUFFIXES.iter().any(|w| n.ends_with(w))
        || NAME_PREFIXES.iter().any(|w| n.starts_with(w))
    {
        return true;
    }
    let host = host.to_lowercase();
    HOST_PARAMS
        .iter()
        .any(|(pattern, names)| host_matches(pattern, &host) && names.contains(&n.as_str()))
}

/// `%XX` decoding (no `+`), lossy on invalid UTF-8.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(0));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Split `s` at every `;` that is followed by a `name=` token (a non-empty run of characters other than `&`, `;` and `=`, then
/// `=`). Any other `;` stays inside its part. The parts keep their text; the `;` themselves are dropped.
#[must_use]
pub fn split_params(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b != b';' {
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && !matches!(bytes[j], b'&' | b';' | b'=') {
            j += 1;
        }
        if j > i + 1 && j < bytes.len() && bytes[j] == b'=' {
            parts.push(&s[start..i]);
            start = i + 1;
        }
    }
    parts.push(&s[start..]);
    parts
}

/// The raw query with every separating `;` turned into `&`, ready for form decoding.
#[must_use]
pub fn query_with_ampersands(query: &str) -> String {
    split_params(query).join("&")
}

/// Remove path parameters (`/job/123;jsessionid=ABC`) the rules drop, with their `;`. The path is otherwise untouched.
#[must_use]
pub fn strip_path_params(host: &str, path: &str) -> String {
    if !path.contains(';') {
        return path.to_string();
    }
    path.split('/')
        .map(|segment| {
            let mut parts = split_params(segment).into_iter();
            let mut out = parts.next().unwrap_or("").to_string();
            for p in parts {
                let (n, v) = p.split_once('=').unwrap_or((p, ""));
                if drop_param(host, &percent_decode(n), &percent_decode(v)) {
                    continue;
                }
                out.push(';');
                out.push_str(p);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Any userinfo, or an email in a path segment, makes the URL personal as a whole. The client and the ingest use this to refuse
/// or flag such a URL; normalization never rewrites the path.
pub fn is_personal_url(input: &str) -> Result<bool, String> {
    let url =
        url::Url::parse(input).map_err(|_| format!("normalizeUrl: not a valid URL: {input}"))?;
    // Any userinfo (`user:pass@host`) is personal, an email in it or not.
    if !url.username().is_empty() || url.password().is_some() {
        return Ok(true);
    }
    Ok(url
        .path()
        .split('/')
        .any(|segment| contains_email(&percent_decode(segment))))
}
