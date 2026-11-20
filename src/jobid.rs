//! The workspace job id of a URL: `normalizeUrl` and `jobIdFromUrl` of `@cv/jobcache/ids`.
//!
//! The rules are frozen (changing them re-keys every job row), and versioned: [`URL_RULE_VERSION`] 2 (ccvl 0.1.12) also drops
//! query parameters that carry personal data or credentials. The same normalization the
//! `jobcache` contract crate mirrors. `job.create` derives the row key from them, so the engine
//! needs them to put a row where the TypeScript applier does. Parity is proven by the vectors
//! (`tests/vectors/v7-data.json`), which come from the real `jobIdFromUrl`.
use sha2::{Digest, Sha256};
use url::Url;

/// The version of the URL rule set. 1: tracking parameters, fragment, trailing slash, unreserved percent-escapes, sorted keys.
/// 2 (ccvl 0.1.12): additionally drops personal-data query parameters, see [`PERSONAL_PARAMS`] and [`is_email_shaped`].
/// The TypeScript twin is `URL_RULE_VERSION` in `@cv/jobcache/ids`; both read `tests/fixtures/normalize-url.json`.
pub const URL_RULE_VERSION: u32 = 2;

/// Query parameters dropped by NAME (case-insensitive) because they carry credentials. `key`, `id`, `uid`, `sid` and `jk`
/// are NOT here on purpose: boards use them as job ids.
const PERSONAL_PARAMS: &[&str] = &[
    "token",
    "access_token",
    "auth",
    "authorization",
    "password",
    "pass",
    "pwd",
    "secret",
    "signature",
    "sig",
    "session",
    "sessionid",
    "apikey",
    "api_key",
];

/// Email-shaped: no ASCII whitespace, exactly one `@`, a non-empty local part, and a domain with a `.` that is neither its
/// first nor its last character. Hand-written (no regex, ASCII-only whitespace) so it matches the TypeScript twin exactly.
#[must_use]
pub fn is_email_shaped(value: &str) -> bool {
    let mut at: Option<usize> = None;
    for (i, ch) in value.char_indices() {
        if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{0c}' | '\u{0b}') {
            return false;
        }
        if ch == '@' {
            if at.is_some() {
                return false;
            }
            at = Some(i);
        }
    }
    let Some(at) = at else { return false };
    if at == 0 {
        return false;
    }
    // A `.` strictly inside the domain: not its first and not its last character.
    let mut domain = value[at + 1..].chars();
    domain.next();
    domain.next_back();
    domain.any(|c| c == '.')
}

/// Tracking-only query parameters stripped during normalization (frozen list).
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "mc_eid",
    "mc_cid",
    "_ga",
    "_gl",
    "ref",
    "referrer",
    "source",
];

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Rewrite `%XX`: an unreserved byte becomes the literal character, any other escape keeps
/// its form with uppercase hex digits.
fn decode_unreserved_percents(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let code = u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(0);
            if is_unreserved(code) {
                out.push(char::from(code));
            } else {
                out.push('%');
                out.push(char::from(bytes[i + 1].to_ascii_uppercase()));
                out.push(char::from(bytes[i + 2].to_ascii_uppercase()));
            }
            i += 3;
            continue;
        }
        // The path is ASCII after URL parsing; copy a whole character regardless.
        let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `normalizeUrl`: the error text is the TypeScript one.
pub fn normalize_url(input: &str) -> Result<String, String> {
    if input.is_empty() {
        return Err("normalizeUrl: input must be a non-empty string, got string".into());
    }
    let mut url =
        Url::parse(input).map_err(|_| format!("normalizeUrl: not a valid URL: {input}"))?;
    // The TypeScript lowercases the host explicitly (`parsed.host = parsed.host.toLowerCase()`):
    // a non-special scheme such as `careervector-local://Host/id` keeps its host case otherwise.
    if let Some(host) = url.host_str() {
        let lower = host.to_lowercase();
        if lower != host {
            url.set_host(Some(&lower))
                .map_err(|_| format!("normalizeUrl: not a valid URL: {input}"))?;
        }
    }
    url.set_fragment(None);

    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut keys: Vec<&String> = Vec::new();
    for (k, _) in &pairs {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    // `Array.prototype.sort()` orders by UTF-16 code units.
    keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let mut kept: Vec<(&str, &str)> = Vec::new();
    for key in keys {
        let lower = key.to_lowercase();
        if TRACKING_PARAMS.contains(&lower.as_str()) || PERSONAL_PARAMS.contains(&lower.as_str()) {
            continue;
        }
        for (k, v) in &pairs {
            if k == key && !is_email_shaped(v) {
                kept.push((k, v));
            }
        }
    }
    if kept.is_empty() {
        url.set_query(None);
    } else {
        let mut q = url.query_pairs_mut();
        q.clear();
        for (k, v) in &kept {
            q.append_pair(k, v);
        }
    }

    let path = url.path().to_string();
    if path.len() > 1 && path.ends_with('/') {
        let trimmed = path.trim_end_matches('/');
        url.set_path(if trimmed.is_empty() { "/" } else { trimmed });
    }
    let decoded = decode_unreserved_percents(url.path());
    url.set_path(&decoded);
    Ok(url.to_string())
}

/// SHA-256, first 10 bytes, RFC 4648 base32, lowercase, no padding (16 characters).
fn hash_text(input: &str) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(16);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for &byte in &digest[..10] {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
        buffer &= (1 << bits) - 1;
    }
    out
}

/// `jobIdFromUrl`.
pub fn job_id_from_url(url: &str) -> Result<String, String> {
    let normalized = normalize_url(url)?;
    Ok(format!("job_{}", hash_text(&format!("job:{normalized}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_ignore_tracking() {
        let a = job_id_from_url("https://Example.com/jobs/1/?utm_source=x&b=2&a=1#frag").unwrap();
        let b = job_id_from_url("https://example.com/jobs/1?a=1&b=2").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 4 + 16);
    }

    #[test]
    fn host_is_lowercased_for_every_scheme_like_the_typescript() {
        // Vectors from the real `normalizeUrl`/`jobIdFromUrl` (@cv/jobcache/ids).
        assert_eq!(
            normalize_url("careervector-local://Foo_Bar/ABC-1").unwrap(),
            "careervector-local://foo_bar/ABC-1"
        );
        assert_eq!(
            job_id_from_url("careervector-local://Foo_Bar/ABC-1").unwrap(),
            "job_e4cq2vew7qbsvzhp"
        );
        assert_eq!(
            job_id_from_url("careervector-local://FooBar:80/x").unwrap(),
            "job_2fapclykhvqdchcw"
        );
        assert_eq!(
            normalize_url("foo://User@Host.Example/Path").unwrap(),
            "foo://User@host.example/Path"
        );
    }

    #[test]
    fn refuses_what_does_not_parse() {
        assert_eq!(
            job_id_from_url("not a url").unwrap_err(),
            "normalizeUrl: not a valid URL: not a url"
        );
    }
}
