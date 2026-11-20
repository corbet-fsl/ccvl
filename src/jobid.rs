//! The workspace job id of a URL: `normalizeUrl` and `jobIdFromUrl` of `@cv/jobcache/ids`.
//!
//! The rules are frozen (changing them re-keys every job row), and versioned: [`URL_RULE_VERSION`] 3 also drops query and path
//! parameters that carry personal data or credentials (`crate::url_rules`). The same normalization the
//! `jobcache` contract crate mirrors. `job.create` derives the row key from them, so the engine
//! needs them to put a row where the TypeScript applier does. Parity is proven by the vectors
//! (`tests/vectors/v7-data.json`), which come from the real `jobIdFromUrl`.
use crate::url_rules::{drop_param, strip_path_params};
use sha2::{Digest, Sha256};
use url::Url;

pub use crate::url_rules::{URL_RULE_VERSION, contains_email, is_personal_url};

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

    // Query parameters are separated by `&` and `;`. The parsing is the URL standard's form decoding (`+` is a space, `%XX` is
    // decoded) on the raw query, so a URL without `;` yields exactly the pairs `query_pairs()` gave before.
    let host = url.host_str().unwrap_or("").to_string();
    let raw_query = url.query().unwrap_or("").replace(';', "&");
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(raw_query.as_bytes())
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
        if TRACKING_PARAMS.contains(&key.to_lowercase().as_str()) {
            continue;
        }
        for (k, v) in &pairs {
            if k == key && !drop_param(&host, k, v) {
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

    let stripped = strip_path_params(&host, url.path());
    if stripped != url.path() {
        url.set_path(&stripped);
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
