//! Minimal tar and JSON helpers shared by the maintainer release routes.
//!
//! The release and private downstream checks read `git archive` output and
//! write the source bundle. Both need only regular files, so this module
//! implements exactly the ustar and pax subset involved instead of adding a
//! general archive dependency.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use flate2::{Compression, write::GzEncoder};
use serde_json::Value;
use sha2::{Digest, Sha256};

const BLOCK: usize = 512;

/// Environment captured once so checks stay testable without touching the
/// process environment.
#[derive(Clone, Debug, Default)]
pub struct Env(BTreeMap<String, String>);

impl Env {
    #[must_use]
    pub fn from_process() -> Self {
        Self(std::env::vars().collect())
    }

    #[must_use]
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self(
            pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.0.insert(key.to_owned(), value.to_owned());
    }

    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }

    /// The raw value, empty when set to an empty string.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// The value only when it is set and not empty.
    #[must_use]
    pub fn nonempty(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|value| !value.is_empty())
    }

    /// First set and non empty variable of `keys`.
    #[must_use]
    pub fn first_nonempty(&self, keys: &[&str]) -> Option<&str> {
        keys.iter().find_map(|key| self.nonempty(key))
    }

    pub fn pairs(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }
}

/// Kind of an archive member as far as source verification cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Other,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub mode: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Archive {
    /// The `comment` record of a global pax header: `git archive` stores the
    /// commit there.
    pub comment: Option<String>,
    pub entries: Vec<Entry>,
}

#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// The SHA-256 object id and size a Git LFS pointer file names. Any other
/// content, including a malformed pointer, is not one.
#[must_use]
pub fn lfs_pointer(data: &[u8]) -> Option<(String, u64)> {
    if data.len() > 1024 {
        return None;
    }
    let mut lines = std::str::from_utf8(data).ok()?.lines();
    if lines.next()? != "version https://git-lfs.github.com/spec/v1" {
        return None;
    }
    let (mut oid, mut size) = (None, None);
    for line in lines {
        if let Some(value) = line.strip_prefix("oid sha256:") {
            oid = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("size ") {
            size = value.parse().ok();
        }
    }
    let oid = oid?;
    let hex = oid.len() == 64
        && oid
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    hex.then_some((oid, size?))
}

pub fn file_sha256(path: &Path) -> Result<String> {
    let mut stream =
        fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut hash = Sha256::new();
    std::io::copy(&mut stream, &mut hash)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(format!("{:x}", hash.finalize()))
}

fn octal(field: &[u8]) -> Result<u64> {
    if field.first().is_some_and(|byte| byte & 0x80 != 0) {
        // Base 256 numbers for sizes that do not fit the octal field.
        let mut value = u64::from(field[0] & 0x7f);
        for byte in &field[1..] {
            value = value
                .checked_mul(256)
                .and_then(|value| value.checked_add(u64::from(*byte)))
                .context("tar number is too large")?;
        }
        return Ok(value);
    }
    let text: String = field
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| char::from(*byte))
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).context("invalid tar number")
}

fn text(field: &[u8]) -> String {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Parse `<length> <key>=<value>\n` pax records.
fn pax_records(data: &[u8]) -> Result<Vec<(String, String)>> {
    let mut records = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let space = rest
            .iter()
            .position(|byte| *byte == b' ')
            .context("invalid pax record")?;
        let length: usize = std::str::from_utf8(&rest[..space])
            .ok()
            .and_then(|digits| digits.parse().ok())
            .context("invalid pax record length")?;
        if length <= space + 1 || length > rest.len() {
            bail!("invalid pax record length");
        }
        let body = &rest[space + 1..length];
        let body = body.strip_suffix(b"\n").context("invalid pax record")?;
        let equals = body
            .iter()
            .position(|byte| *byte == b'=')
            .context("invalid pax record")?;
        records.push((
            String::from_utf8_lossy(&body[..equals]).into_owned(),
            String::from_utf8_lossy(&body[equals + 1..]).into_owned(),
        ));
        rest = &rest[length..];
    }
    Ok(records)
}

/// Read an uncompressed tar archive.
pub fn read(data: &[u8]) -> Result<Archive> {
    let mut archive = Archive::default();
    let mut offset = 0;
    let mut next_path: Option<String> = None;
    let mut next_size: Option<u64> = None;
    while offset + BLOCK <= data.len() {
        let header = &data[offset..offset + BLOCK];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let stored = octal(&header[148..156])?;
        let computed: u64 = header
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                if (148..156).contains(&index) {
                    u64::from(b' ')
                } else {
                    u64::from(*byte)
                }
            })
            .sum();
        if stored != computed {
            bail!("bad checksum in tar header");
        }
        let mut size = octal(&header[124..136])?;
        let kind = header[156];
        let mode = u32::try_from(octal(&header[100..108])?).context("invalid tar mode")?;
        let mut name = text(&header[0..100]);
        if header[257..262] == *b"ustar" {
            let prefix = text(&header[345..500]);
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        if matches!(kind, b'x' | b'g' | b'L') {
            // Metadata members carry their own payload size.
        } else if let Some(extended) = next_size.take() {
            size = extended;
        }
        let size = usize::try_from(size).context("tar member is too large")?;
        let start = offset + BLOCK;
        let end = start.checked_add(size).context("tar member is too large")?;
        if end > data.len() {
            bail!("unexpected end of tar archive");
        }
        let payload = &data[start..end];
        offset = start + size.div_ceil(BLOCK) * BLOCK;
        match kind {
            b'g' => {
                for (key, value) in pax_records(payload)? {
                    if key == "comment" {
                        archive.comment = Some(value);
                    }
                }
            }
            b'x' => {
                for (key, value) in pax_records(payload)? {
                    match key.as_str() {
                        "path" => next_path = Some(value),
                        "size" => next_size = value.parse().ok(),
                        _ => {}
                    }
                }
            }
            b'L' => next_path = Some(text(payload)),
            _ => {
                if let Some(path) = next_path.take() {
                    name = path;
                }
                let (kind, name) = match kind {
                    b'0' | 0 | b'7' => (Kind::File, name),
                    b'5' => (Kind::Directory, name.trim_end_matches('/').to_owned()),
                    _ => (Kind::Other, name),
                };
                archive.entries.push(Entry {
                    name,
                    kind,
                    mode,
                    data: if kind == Kind::File {
                        payload.to_vec()
                    } else {
                        Vec::new()
                    },
                });
            }
        }
    }
    Ok(archive)
}

fn put(header: &mut [u8; BLOCK], start: usize, field: usize, value: &[u8]) {
    header[start..start + value.len().min(field)].copy_from_slice(&value[..value.len().min(field)]);
}

fn header(name: &[u8], size: usize, mode: u32, kind: u8) -> [u8; BLOCK] {
    let mut header = [0_u8; BLOCK];
    put(&mut header, 0, 100, name);
    put(&mut header, 100, 8, format!("{mode:07o}\0").as_bytes());
    put(&mut header, 108, 8, b"0000000\0");
    put(&mut header, 116, 8, b"0000000\0");
    put(&mut header, 124, 12, format!("{size:011o}\0").as_bytes());
    put(&mut header, 136, 12, b"00000000000\0");
    header[148..156].fill(b' ');
    header[156] = kind;
    put(&mut header, 257, 6, b"ustar\0");
    put(&mut header, 263, 2, b"00");
    let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    put(&mut header, 148, 8, format!("{checksum:06o}\0 ").as_bytes());
    header
}

/// One `<length> <key>=<value>` record; the length counts its own digits.
fn pax_record(key: &str, value: &str) -> String {
    let body = format!(" {key}={value}\n");
    let mut length = body.len() + 1;
    while length.to_string().len() + body.len() != length {
        length = length.to_string().len() + body.len();
    }
    format!("{length}{body}")
}

fn push_member(out: &mut Vec<u8>, name: &str, mode: u32, kind: u8, data: &[u8]) {
    if name.len() > 100 {
        let record = pax_record("path", name);
        out.extend_from_slice(&header(b"PaxHeaders/entry", record.len(), 0o644, b'x'));
        out.extend_from_slice(record.as_bytes());
        out.resize(out.len().next_multiple_of(BLOCK), 0);
    }
    out.extend_from_slice(&header(name.as_bytes(), data.len(), mode, kind));
    out.extend_from_slice(data);
    out.resize(out.len().next_multiple_of(BLOCK), 0);
}

/// Serialize regular files as a tar archive with zeroed times and owners.
#[must_use]
pub fn write<'a>(files: impl IntoIterator<Item = (&'a str, u32, &'a [u8])>) -> Vec<u8> {
    write_with_comment(None, files)
}

/// Like [`write`], optionally with the global pax `comment` record that
/// the source archive format uses to name the source commit.
#[must_use]
pub fn write_with_comment<'a>(
    comment: Option<&str>,
    files: impl IntoIterator<Item = (&'a str, u32, &'a [u8])>,
) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(comment) = comment {
        let record = pax_record("comment", comment);
        out.extend_from_slice(&header(b"pax_global_header", record.len(), 0o644, b'g'));
        out.extend_from_slice(record.as_bytes());
        out.resize(out.len().next_multiple_of(BLOCK), 0);
    }
    for (name, mode, data) in files {
        push_member(&mut out, name, mode, b'0', data);
    }
    out.extend_from_slice(&[0; BLOCK * 2]);
    out
}

/// Write `files` as a gzip compressed tar archive at `path`.
pub fn write_tar_gz<'a>(
    path: &Path,
    files: impl IntoIterator<Item = (&'a str, u32, &'a [u8])>,
) -> Result<()> {
    let output =
        fs::File::create_new(path).with_context(|| format!("cannot create {}", path.display()))?;
    let mut encoder = GzEncoder::new(output, Compression::best());
    encoder.write_all(&write(files))?;
    encoder.finish()?.flush()?;
    Ok(())
}

/// Collapse a name the way `PurePosixPath` does and report whether it is
/// absolute or contains parent or repository directories.
#[must_use]
pub fn unsafe_member_path(name: &str) -> bool {
    name.starts_with('/') || name.split('/').any(|part| part == ".." || part == ".git")
}

/// Run a command and return its trimmed standard output.
pub fn command_text(root: &Path, program: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("cannot run {}", program.display()))?;
    if !output.status.success() {
        bail!(
            "Command '{} {}' failed with {}.",
            program.display(),
            args.join(" "),
            output.status
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Resolve a possibly relative path against the current directory without
/// requiring it to exist.
pub fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))
}

fn escape(value: &str, out: &mut String) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(character),
            _ => {
                let mut units = [0_u16; 2];
                for unit in character.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

fn dump(value: &Value, indent: Option<usize>, level: usize, out: &mut String) {
    let newline = |out: &mut String, level: usize| {
        if let Some(width) = indent {
            out.push('\n');
            out.push_str(&" ".repeat(width * level));
        }
    };
    let separator = if indent.is_some() { "," } else { ", " };
    match value {
        Value::Array(items) if !items.is_empty() => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(separator);
                }
                newline(out, level + 1);
                dump(item, indent, level + 1, out);
            }
            newline(out, level);
            out.push(']');
        }
        Value::Object(map) if !map.is_empty() => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push_str(separator);
                }
                newline(out, level + 1);
                escape(key, out);
                out.push_str(": ");
                dump(&map[key], indent, level + 1, out);
            }
            newline(out, level);
            out.push('}');
        }
        Value::Array(_) => out.push_str("[]"),
        Value::Object(_) => out.push_str("{}"),
        Value::String(text) => escape(text, out),
        other => out.push_str(&other.to_string()),
    }
}

/// Serialize like `json.dumps(value, sort_keys=True)`; with `indent`, like
/// `json.dumps(value, indent=n, sort_keys=True)`.
#[must_use]
pub fn dumps(value: &Value, indent: Option<usize>) -> String {
    let mut out = String::new();
    dump(value, indent, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn lfs_pointers_are_recognised_exactly() {
        let oid = "ab".repeat(32);
        let pointer =
            format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize 12\n");
        assert_eq!(lfs_pointer(pointer.as_bytes()), Some((oid.clone(), 12)));
        for broken in [
            String::new(),
            "version https://example.invalid/spec/v1\n".to_owned(),
            format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 12\n",
                "AB".repeat(32)
            ),
            format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 12\n",
                "ab".repeat(31)
            ),
            format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\n"),
            format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize x\n"),
            format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize 1\n{}",
                "x".repeat(2000)
            ),
        ] {
            assert_eq!(lfs_pointer(broken.as_bytes()), None, "{broken:?}");
        }
        assert_eq!(lfs_pointer(&[0xff, 0xfe]), None);
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn json_matches_the_established_evidence_format() {
        let value = json!({"b": [1, "é"], "a": {"z": true, "y": null}, "c": [], "d": {}});
        assert_eq!(
            dumps(&value, None),
            r#"{"a": {"y": null, "z": true}, "b": [1, "\u00e9"], "c": [], "d": {}}"#
        );
        assert_eq!(
            dumps(&value, Some(2)),
            "{\n  \"a\": {\n    \"y\": null,\n    \"z\": true\n  },\n  \"b\": [\n    1,\n    \"\\u00e9\"\n  ],\n  \"c\": [],\n  \"d\": {}\n}"
        );
    }

    #[test]
    fn written_archives_round_trip_including_long_names() {
        let long = format!("{}/file.txt", "directory".repeat(20));
        let bytes = write([
            ("a.txt", 0o644, b"alpha".as_slice()),
            (long.as_str(), 0o755, b"beta".as_slice()),
        ]);
        let archive = read(&bytes).unwrap();
        assert_eq!(archive.entries.len(), 2);
        assert_eq!(archive.entries[0].name, "a.txt");
        assert_eq!(archive.entries[0].data, b"alpha");
        assert_eq!(archive.entries[1].name, long);
        assert_eq!(archive.entries[1].mode, 0o755);
        assert_eq!(archive.entries[1].data, b"beta");
    }

    #[test]
    fn unsafe_member_names_are_detected() {
        assert!(unsafe_member_path("/absolute"));
        assert!(unsafe_member_path("a/../b"));
        assert!(unsafe_member_path("a/.git/config"));
        assert!(!unsafe_member_path(".agent/src/main.rs"));
        assert!(!unsafe_member_path(".gitattributes"));
    }
}
