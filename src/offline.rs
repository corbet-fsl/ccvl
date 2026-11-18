//! The offline merge policy: what happens to an edit that was made while the device was away.
//!
//! Two facts decide it, both known at reconnect:
//! - **age**: arrival time minus the time the edit was **made** (a hybrid logical clock, see [`Hlc`]);
//! - **conflict**: the field's value elsewhere differs from the value the edit was made on (its base).
//!
//! | Case | Outcome |
//! |---|---|
//! | no conflict, any age | [`Resolution::Apply`] |
//! | conflict, fresh (age <= [`STALE_AFTER_MS`]), text | [`Resolution::Merged`] (only the changed range is applied) |
//! | conflict, fresh, single value | [`Resolution::Apply`] (the engine's deterministic order converges every device) |
//! | conflict, stale | [`Resolution::Keep`]: never applied silently, never discarded |
//!
//! No I/O and no dependencies: the web (wasm), the TUI and the CLI call the same functions. The JSON
//! vectors in `tests/vectors/offline.json` are shared with the TypeScript twin used when the core flag is off.

use core::cmp::Ordering;
use core::fmt;

/// An edit older than this (at arrival) that conflicts is kept for review instead of applied. One hour sits
/// above the "briefly offline" cases (Wi-Fi drop, tunnel, train, laptop asleep over lunch) and below the
/// "away" cases (flight, overnight, days).
pub const STALE_AFTER_MS: u64 = 3_600_000;

/// Largest field value kept as a text base for the range merge; bigger values are compared by hash only.
pub const MAX_BASE_BYTES: usize = 16 * 1024;

/// Hybrid logical clock stamp: wall milliseconds plus a counter that orders stamps inside one millisecond
/// and keeps the clock monotonic when the wall clock steps back. Compares by `(wall, counter)`; the text
/// form (`wwwwwwwwwwwww-cccc`, lowercase hex) sorts like the stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hlc {
    /// Wall-clock milliseconds since the Unix epoch (52 bits).
    pub wall: u64,
    /// Tie-breaker inside one wall millisecond.
    pub counter: u16,
}

impl Hlc {
    /// Largest wall value the text form carries (13 hex digits).
    pub const MAX_WALL: u64 = (1 << 52) - 1;

    /// The next stamp for a local event at wall time `now`. Strictly greater than `self`.
    #[must_use]
    pub fn tick(self, now: u64) -> Hlc {
        let now = now.min(Self::MAX_WALL);
        if now > self.wall {
            Hlc {
                wall: now,
                counter: 0,
            }
        } else {
            self.bump()
        }
    }

    /// The next stamp after seeing `remote` at wall time `now`: greater than both `self` and `remote`.
    #[must_use]
    pub fn observe(self, now: u64, remote: Hlc) -> Hlc {
        let top = self.max(remote);
        let now = now.min(Self::MAX_WALL);
        if now > top.wall {
            Hlc {
                wall: now,
                counter: 0,
            }
        } else {
            top.bump()
        }
    }

    fn bump(self) -> Hlc {
        match self.counter.checked_add(1) {
            Some(counter) => Hlc {
                wall: self.wall,
                counter,
            },
            None => Hlc {
                wall: self.wall.saturating_add(1).min(Self::MAX_WALL),
                counter: 0,
            },
        }
    }

    /// Age of an edit made at this stamp, seen at wall time `now`. A stamp from the future (clock skew)
    /// has age 0: it counts as fresh, never as stale.
    #[must_use]
    pub fn age_ms(self, now: u64) -> u64 {
        now.saturating_sub(self.wall)
    }

    /// Parse the text form; `None` for anything else, so an old client's op (no stamp) is "unknown".
    #[must_use]
    pub fn parse(s: &str) -> Option<Hlc> {
        let (w, c) = s.split_once('-')?;
        if w.len() != 13 || c.len() != 4 {
            return None;
        }
        if !w
            .bytes()
            .chain(c.bytes())
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        Some(Hlc {
            wall: u64::from_str_radix(w, 16).ok()?,
            counter: u16::from_str_radix(c, 16).ok()?,
        })
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.wall, self.counter).cmp(&(other.wall, other.counter))
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:013x}-{:04x}",
            self.wall.min(Self::MAX_WALL),
            self.counter
        )
    }
}

/// How a field merges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Free text: concurrent edits to different ranges both survive.
    Text,
    /// One value (status, date, number, choice): there is no merging two values.
    Single,
}

/// What to do with one queued edit at reconnect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Send the edit as made.
    Apply,
    /// Send this text instead: the edit's change laid over what changed elsewhere.
    Merged(String),
    /// Do not send. Roll the field back to the current value and keep the edit for the person to review.
    Keep,
    /// Do not send and do not keep: the same single value was changed elsewhere later (see [`decide_single`]).
    Drop,
}

/// Decide one edit. `base` is the field value the edit was made on, `mine` the value it produced, `theirs`
/// the value now elsewhere; `age_ms` is [`Hlc::age_ms`] of the edit. Values are the field's text, or its
/// JSON text for single-value fields.
#[must_use]
pub fn resolve(kind: FieldKind, base: &str, mine: &str, theirs: &str, age_ms: u64) -> Resolution {
    if theirs == base || theirs == mine {
        return Resolution::Apply;
    }
    if age_ms > STALE_AFTER_MS {
        return Resolution::Keep;
    }
    match kind {
        FieldKind::Single => Resolution::Apply,
        FieldKind::Text => match merge_text(base, mine, theirs) {
            Some(merged) => Resolution::Merged(merged),
            None => Resolution::Keep,
        },
    }
}

/// A fresh conflict on a single value: the change made last wins. `mine_made_ms` is when the edit was made;
/// `theirs_made_ms` is when the last change elsewhere was made (its server time, as it was made online), or
/// `None` when no recent change to the field is known: such a change is older than any fresh edit, so mine wins.
/// Equal times keep mine (the edit that waited is not punished for a tie).
#[must_use]
pub fn decide_single(mine_made_ms: u64, theirs_made_ms: Option<u64>) -> Resolution {
    match theirs_made_ms {
        Some(theirs) if theirs > mine_made_ms => Resolution::Drop,
        _ => Resolution::Apply,
    }
}

/// Three-way merge of two edits of `base` by changed range. The change of each side is the part between the
/// common prefix and the common suffix with `base` (counted in code points). `None` when both changed the
/// same place (ranges overlap, or both insert at one point): that is a real conflict, not a merge.
#[must_use]
pub fn merge_text(base: &str, mine: &str, theirs: &str) -> Option<String> {
    let b: Vec<char> = base.chars().collect();
    let m: Vec<char> = mine.chars().collect();
    let t: Vec<char> = theirs.chars().collect();
    let (ma, mb) = changed_range(&b, &m);
    let (ta, tb) = changed_range(&b, &t);
    let mine_empty = ma == mb && m.len() == b.len();
    let theirs_empty = ta == tb && t.len() == b.len();
    if mine_empty {
        return Some(theirs.to_string());
    }
    if theirs_empty {
        return Some(mine.to_string());
    }
    let mine_first = if mb <= ta && !(ma == mb && ta == tb && ma == ta) {
        true
    } else if tb <= ma && !(ma == mb && ta == tb && ma == ta) {
        false
    } else {
        return None;
    };
    let m_repl = &m[ma..m.len() - (b.len() - mb)];
    let t_repl = &t[ta..t.len() - (b.len() - tb)];
    let (first, second) = if mine_first {
        ((ma, mb, m_repl), (ta, tb, t_repl))
    } else {
        ((ta, tb, t_repl), (ma, mb, m_repl))
    };
    let mut out = String::with_capacity(base.len() + mine.len() + theirs.len());
    out.extend(&b[..first.0]);
    out.extend(first.2);
    out.extend(&b[first.1..second.0]);
    out.extend(second.2);
    out.extend(&b[second.1..]);
    Some(out)
}

/// The range `[start, end)` of `base` that `edited` replaced (prefix and suffix that agree are outside).
fn changed_range(base: &[char], edited: &[char]) -> (usize, usize) {
    let max = base.len().min(edited.len());
    let mut p = 0;
    while p < max && base[p] == edited[p] {
        p += 1;
    }
    let mut s = 0;
    while s < max - p && base[base.len() - 1 - s] == edited[edited.len() - 1 - s] {
        s += 1;
    }
    (p, base.len() - s)
}

/// Whether a base of this size is kept in full (text merge possible) or by hash only.
#[must_use]
pub fn base_fits(value: &str) -> bool {
    value.len() <= MAX_BASE_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRESH: u64 = 60_000;
    const STALE: u64 = STALE_AFTER_MS + 1;

    #[test]
    fn hlc_ticks_monotonically_when_the_clock_steps_back() {
        let a = Hlc::default().tick(1000);
        let b = a.tick(900);
        let c = b.tick(1000);
        assert!(a < b && b < c);
        assert_eq!(b.wall, 1000);
        assert_eq!(
            Hlc::default().tick(5),
            Hlc {
                wall: 5,
                counter: 0
            }
        );
    }

    #[test]
    fn hlc_observe_passes_both_sides() {
        let local = Hlc {
            wall: 100,
            counter: 3,
        };
        let remote = Hlc {
            wall: 200,
            counter: 9,
        };
        let o = local.observe(150, remote);
        assert!(o > local && o > remote);
        assert_eq!(
            local.observe(300, remote),
            Hlc {
                wall: 300,
                counter: 0
            }
        );
        assert_eq!(
            remote.observe(10, local),
            Hlc {
                wall: 200,
                counter: 10
            }
        );
    }

    #[test]
    fn hlc_counter_overflow_moves_the_wall() {
        let a = Hlc {
            wall: 7,
            counter: u16::MAX,
        };
        assert_eq!(
            a.tick(7),
            Hlc {
                wall: 8,
                counter: 0
            }
        );
    }

    #[test]
    fn hlc_text_form_round_trips_and_sorts() {
        let a = Hlc {
            wall: 1_700_000_000_000,
            counter: 2,
        };
        let s = a.to_string();
        assert_eq!(s.len(), 18);
        assert_eq!(Hlc::parse(&s), Some(a));
        let b = Hlc {
            wall: 1_700_000_000_000,
            counter: 10,
        };
        assert!(a.to_string() < b.to_string());
        let c = Hlc {
            wall: 1_700_000_000_001,
            counter: 0,
        };
        assert!(b.to_string() < c.to_string());
    }

    #[test]
    fn hlc_parse_rejects_foreign_text() {
        for bad in [
            "",
            "x",
            "0000000000000-000",
            "0000000000000_0000",
            "000000000000g-0000",
            "0000000000000-00000",
            "0000000000000-ABCD",
        ] {
            assert_eq!(Hlc::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn hlc_wall_clamps_to_52_bits() {
        let a = Hlc::default().tick(u64::MAX);
        assert_eq!(a.wall, Hlc::MAX_WALL);
        assert_eq!(Hlc::parse(&a.to_string()), Some(a));
    }

    #[test]
    fn age_of_a_future_stamp_is_zero() {
        assert_eq!(
            Hlc {
                wall: 500,
                counter: 0
            }
            .age_ms(100),
            0
        );
        assert_eq!(
            Hlc {
                wall: 100,
                counter: 0
            }
            .age_ms(500),
            400
        );
    }

    #[test]
    fn no_conflict_applies_at_any_age() {
        for age in [0, FRESH, STALE, 3 * 24 * 3_600_000] {
            for kind in [FieldKind::Text, FieldKind::Single] {
                assert_eq!(resolve(kind, "a", "b", "a", age), Resolution::Apply);
            }
        }
    }

    #[test]
    fn same_value_elsewhere_applies() {
        assert_eq!(
            resolve(FieldKind::Single, "open", "applied", "applied", STALE),
            Resolution::Apply
        );
    }

    #[test]
    fn fresh_conflict_merges_text_by_range() {
        let r = resolve(
            FieldKind::Text,
            "hello world",
            "Hello world",
            "hello world!",
            FRESH,
        );
        assert_eq!(r, Resolution::Merged("Hello world!".into()));
    }

    #[test]
    fn fresh_overlapping_text_is_kept_not_guessed() {
        assert_eq!(
            resolve(FieldKind::Text, "abc", "aXc", "aYc", FRESH),
            Resolution::Keep
        );
    }

    #[test]
    fn fresh_single_value_conflict_applies() {
        assert_eq!(
            resolve(
                FieldKind::Single,
                "\"open\"",
                "\"applied\"",
                "\"rejected\"",
                FRESH
            ),
            Resolution::Apply
        );
    }

    #[test]
    fn stale_conflict_is_kept_for_both_kinds() {
        assert_eq!(
            resolve(
                FieldKind::Text,
                "hello world",
                "Hello world",
                "hello world!",
                STALE
            ),
            Resolution::Keep
        );
        assert_eq!(
            resolve(FieldKind::Single, "1", "2", "3", STALE),
            Resolution::Keep
        );
    }

    #[test]
    fn the_threshold_itself_is_still_fresh() {
        assert_eq!(
            resolve(FieldKind::Single, "1", "2", "3", STALE_AFTER_MS),
            Resolution::Apply
        );
        assert_eq!(
            resolve(FieldKind::Single, "1", "2", "3", STALE_AFTER_MS + 1),
            Resolution::Keep
        );
    }

    #[test]
    fn merge_disjoint_edits_in_either_order() {
        assert_eq!(
            merge_text("one two three", "ONE two three", "one two THREE").as_deref(),
            Some("ONE two THREE")
        );
        assert_eq!(
            merge_text("one two three", "one two THREE", "ONE two three").as_deref(),
            Some("ONE two THREE")
        );
    }

    #[test]
    fn merge_one_side_unchanged_takes_the_other() {
        assert_eq!(
            merge_text("same", "same", "other").as_deref(),
            Some("other")
        );
        assert_eq!(
            merge_text("same", "other", "same").as_deref(),
            Some("other")
        );
    }

    #[test]
    fn merge_insertions_at_the_same_point_conflict() {
        assert_eq!(merge_text("ab", "aXb", "aYb"), None);
    }

    #[test]
    fn merge_insertions_at_different_points() {
        assert_eq!(merge_text("ab", "Xab", "abY").as_deref(), Some("XabY"));
    }

    #[test]
    fn merge_touching_ranges_merge() {
        assert_eq!(merge_text("abcd", "XXcd", "abYY").as_deref(), Some("XXYY"));
    }

    #[test]
    fn merge_counts_code_points_not_bytes() {
        assert_eq!(
            merge_text("é😀 end", "É😀 end", "é😀 end!").as_deref(),
            Some("É😀 end!")
        );
    }

    #[test]
    fn merge_handles_deletion_and_empty_base() {
        assert_eq!(
            merge_text("abc def", "abc", "ABC def").as_deref(),
            Some("ABC")
        );
        assert_eq!(merge_text("abc def", "abc", "abc DEF").as_deref(), None);
        assert_eq!(merge_text("", "x", "").as_deref(), Some("x"));
        assert_eq!(merge_text("", "x", "y"), None);
    }

    #[test]
    fn single_value_race_is_decided_by_made_time() {
        assert_eq!(decide_single(100, Some(200)), Resolution::Drop);
        assert_eq!(decide_single(200, Some(100)), Resolution::Apply);
        assert_eq!(decide_single(100, Some(100)), Resolution::Apply);
        assert_eq!(decide_single(100, None), Resolution::Apply);
    }

    #[test]
    fn base_size_cap() {
        assert!(base_fits(&"a".repeat(MAX_BASE_BYTES)));
        assert!(!base_fits(&"a".repeat(MAX_BASE_BYTES + 1)));
    }
}
