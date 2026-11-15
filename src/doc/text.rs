//! Minimal prose edits on UTF-16 offsets (the unit Yjs counts text in).

/// One contiguous replacement: remove `remove_len` units at `position`, then insert `insert`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MiddleEdit {
    pub position: u32,
    pub remove_len: u32,
    pub insert: String,
}

const fn is_high_surrogate(unit: u16) -> bool {
    matches!(unit, 0xD800..=0xDBFF)
}

const fn is_low_surrogate(unit: u16) -> bool {
    matches!(unit, 0xDC00..=0xDFFF)
}

fn units_to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Strip the longest common prefix and suffix; the residue is the edit. Same
/// algorithm as `computeMiddleEdit` in `@cv/mutations`, with one difference:
/// the cut never splits a surrogate pair, so the inserted text is always valid
/// (the TypeScript version can insert a lone surrogate for two emoji that share
/// a high surrogate).
pub(crate) fn compute_middle_edit(old: &str, new: &str) -> MiddleEdit {
    let old: Vec<u16> = old.encode_utf16().collect();
    let new: Vec<u16> = new.encode_utf16().collect();
    if old == new {
        return MiddleEdit {
            position: 0,
            remove_len: 0,
            insert: String::new(),
        };
    }
    let mut prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    if prefix > 0 && is_high_surrogate(old[prefix - 1]) {
        prefix -= 1;
    }
    let max_suffix = (old.len() - prefix).min(new.len() - prefix);
    let mut suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    if suffix > 0 && is_low_surrogate(old[old.len() - suffix]) {
        suffix -= 1;
    }
    MiddleEdit {
        position: units_to_u32(prefix),
        remove_len: units_to_u32(old.len() - prefix - suffix),
        insert: String::from_utf16_lossy(&new[prefix..new.len() - suffix]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_insert_and_delete() {
        assert_eq!(
            compute_middle_edit("abc", "abXc"),
            MiddleEdit {
                position: 2,
                remove_len: 0,
                insert: "X".into()
            }
        );
        assert_eq!(
            compute_middle_edit("abXc", "abc"),
            MiddleEdit {
                position: 2,
                remove_len: 1,
                insert: String::new()
            }
        );
        assert_eq!(compute_middle_edit("same", "same").remove_len, 0);
    }

    #[test]
    fn offsets_count_utf16_units() {
        // U+1F600 is two UTF-16 units: the insert lands after both.
        let edit = compute_middle_edit("a\u{1F600}b", "a\u{1F600}Xb");
        assert_eq!(edit.position, 3);
        assert_eq!(edit.insert, "X");
    }

    #[test]
    fn never_splits_a_surrogate_pair() {
        // U+1F600 and U+1F601 share the high surrogate.
        let edit = compute_middle_edit("\u{1F600}", "\u{1F601}");
        assert_eq!(edit.position, 0);
        assert_eq!(edit.remove_len, 2);
        assert_eq!(edit.insert, "\u{1F601}");
    }
}
