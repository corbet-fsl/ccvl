//! JavaScript number and string semantics that the TypeScript reference (and the `SheetJS` library it
//! calls) rely on. The CSV bytes and the import preview must match it exactly, so the few places
//! where Rust and JavaScript disagree are spelled out here instead of approximated.

/// `String.prototype.trim` / `Number(string)` white space: `WhiteSpace` and `LineTerminator`.
#[must_use]
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `String.prototype.trim`.
#[must_use]
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// The shortest digits that read back as `|v|` and the decimal exponent of the first digit. Where
/// two candidates sit exactly equally far from the value, JavaScript takes the even one; Rust's
/// formatter may take the other.
fn shortest_digits(v: f64) -> (Vec<u8>, i32) {
    let sci = format!("{:e}", v.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let mut exp: i32 = exp.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mantissa.bytes().filter(|b| *b != b'.').collect();
    let k = digits.len();
    // The exact expansion of a tie ends in a 5 right after the k-th digit.
    let exact = format!("{:.*e}", k + 40, v.abs());
    let (em, _) = exact.split_once('e').unwrap_or((&exact, "0"));
    let exact_digits: Vec<u8> = em.bytes().filter(|b| *b != b'.').collect();
    let trimmed_len = exact_digits
        .iter()
        .rposition(|b| *b != b'0')
        .map_or(0, |i| i + 1);
    if trimmed_len == k + 1 && exact_digits[k] == b'5' {
        let lower: Vec<u8> = exact_digits[..k].to_vec();
        let mut upper = lower.clone();
        let carried = increment_digits(&mut upper);
        let (upper, upper_exp) = if carried {
            upper.pop();
            (upper, exp + 1)
        } else {
            (upper, exp)
        };
        let reads_back = |d: &[u8], e: i32| {
            let all = String::from_utf8_lossy(d);
            let text = format!("{}.{}e{e}", &all[..1], &all[1..]);
            text.parse::<f64>().is_ok_and(|x| x == v.abs())
        };
        let lower_ok = reads_back(&lower, exp);
        let upper_ok = reads_back(&upper, upper_exp);
        if lower_ok && upper_ok {
            if lower.last().is_some_and(|d| (d - b'0').is_multiple_of(2)) {
                digits = lower;
            } else {
                digits = upper;
                exp = upper_exp;
            }
        }
    }
    (digits, exp)
}

/// `Number::toString(10)` for any finite or non-finite double.
#[must_use]
pub fn number_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_owned();
    }
    if v == 0.0 {
        return "0".to_owned();
    }
    if v.is_infinite() {
        return if v < 0.0 { "-Infinity" } else { "Infinity" }.to_owned();
    }
    let sign = if v < 0.0 { "-" } else { "" };
    let (digits, exp) = shortest_digits(v);
    let digits = String::from_utf8(digits).unwrap_or_default();
    let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    let n = exp + 1;
    let body = if k <= n && n <= 21 {
        format!(
            "{digits}{}",
            "0".repeat(usize::try_from(n - k).unwrap_or(0))
        )
    } else if 0 < n && n <= 21 {
        let at = usize::try_from(n).unwrap_or(0);
        format!("{}.{}", &digits[..at], &digits[at..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat(usize::try_from(-n).unwrap_or(0)))
    } else {
        let e = n - 1;
        let exp_sign = if e < 0 { '-' } else { '+' };
        let head = &digits[..1];
        let tail = &digits[1..];
        if tail.is_empty() {
            format!("{head}e{exp_sign}{}", e.abs())
        } else {
            format!("{head}.{tail}e{exp_sign}{}", e.abs())
        }
    };
    format!("{sign}{body}")
}

/// Add one unit in the last place to a string of decimal digits; `true` when the carry added a digit.
fn increment_digits(digits: &mut Vec<u8>) -> bool {
    for d in digits.iter_mut().rev() {
        if *d == b'9' {
            *d = b'0';
        } else {
            *d += 1;
            return false;
        }
    }
    digits.insert(0, b'1');
    true
}

/// The first `significant` digits of `|v|` (`v != 0`) and its decimal exponent, rounded half up on
/// the exact binary value, which is how `toPrecision`, `toExponential` and `toFixed` pick between
/// two candidates (Rust rounds exact ties to even).
fn significant_digits(v: f64, significant: usize) -> (Vec<u8>, i32) {
    let sci = format!("{:.*e}", significant - 1 + 80, v.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let mut exp: i32 = exp.parse().unwrap_or(0);
    let all: Vec<u8> = mantissa.bytes().filter(|b| *b != b'.').collect();
    let mut head = all[..significant].to_vec();
    if all[significant] >= b'5' && increment_digits(&mut head) {
        head.pop();
        exp += 1;
    }
    (head, exp)
}

/// `Number.prototype.toFixed`.
fn to_fixed(v: f64, digits: usize) -> String {
    if v.abs() >= 1e21 {
        return number_to_string(v);
    }
    let negative = v < 0.0;
    let exact = format!("{:.*}", digits + 80, v.abs());
    let (int_part, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let mut all: Vec<u8> = int_part.bytes().chain(frac.bytes().take(digits)).collect();
    let round_up = frac.as_bytes().get(digits).is_some_and(|d| *d >= b'5');
    let mut int_len = int_part.len();
    if round_up && increment_digits(&mut all) {
        int_len += 1;
    }
    let text = String::from_utf8(all).unwrap_or_default();
    let (i, f) = text.split_at(int_len);
    let body = if digits == 0 {
        i.to_owned()
    } else {
        format!("{i}.{f}")
    };
    if negative { format!("-{body}") } else { body }
}

/// `Number.prototype.toExponential` with an explicit digit count.
fn to_exponential(v: f64, digits: usize) -> String {
    let (d, exp) = if v == 0.0 {
        (vec![b'0'; digits + 1], 0)
    } else {
        significant_digits(v, digits + 1)
    };
    let sign = if v < 0.0 { "-" } else { "" };
    let text = String::from_utf8(d).unwrap_or_default();
    let mantissa = if digits == 0 {
        text
    } else {
        format!("{}.{}", &text[..1], &text[1..])
    };
    format!(
        "{sign}{mantissa}e{}{}",
        if exp < 0 { '-' } else { '+' },
        exp.abs()
    )
}

/// `Number.prototype.toPrecision`.
fn to_precision(v: f64, precision: usize) -> String {
    if v == 0.0 {
        return if precision > 1 {
            format!("0.{}", "0".repeat(precision - 1))
        } else {
            "0".to_owned()
        };
    }
    let (d, e) = significant_digits(v, precision);
    let text = String::from_utf8(d).unwrap_or_default();
    let sign = if v < 0.0 { "-" } else { "" };
    let p = i32::try_from(precision).unwrap_or(i32::MAX);
    if e < -6 || e >= p {
        let mantissa = if precision == 1 {
            text
        } else {
            format!("{}.{}", &text[..1], &text[1..])
        };
        return format!(
            "{sign}{mantissa}e{}{}",
            if e < 0 { '-' } else { '+' },
            e.abs()
        );
    }
    if e >= 0 {
        let int_len = usize::try_from(e + 1).unwrap_or(1);
        return if int_len >= text.len() {
            format!("{sign}{text}")
        } else {
            format!("{sign}{}.{}", &text[..int_len], &text[int_len..])
        };
    }
    format!(
        "{sign}0.{}{text}",
        "0".repeat(usize::try_from(-(e + 1)).unwrap_or(0))
    )
}

/// `SSF_strip_decimal`: drop trailing zeros of a decimal fraction that runs to the end of the text.
fn strip_decimal(s: &str) -> String {
    let Some(dot) = s.find('.') else {
        return s.to_owned();
    };
    let (head, fraction) = s.split_at(dot);
    let digits = &fraction[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return s.to_owned();
    }
    let kept = digits.trim_end_matches('0');
    if kept.is_empty() {
        head.to_owned()
    } else {
        format!("{head}.{kept}")
    }
}

/// `SSF_normalize_exp`: two-digit exponent, trimmed mantissa.
fn normalize_exp(s: &str) -> String {
    let Some(at) = s.find('E') else {
        return s.to_owned();
    };
    let (mantissa, exp) = s.split_at(at);
    let mantissa = strip_decimal(mantissa);
    let exp = &exp[1..];
    let (sign, digits) = exp.split_at(1);
    let digits = if digits.len() == 1 {
        format!("0{digits}")
    } else {
        digits.to_owned()
    };
    format!("{mantissa}E{sign}{digits}")
}

fn small_exp(v: f64) -> String {
    let w = if v < 0.0 { 12 } else { 11 };
    let o = strip_decimal(&to_fixed(v, 12));
    if o.len() <= w {
        return o;
    }
    let o = to_precision(v, 10);
    if o.len() <= w {
        return o;
    }
    to_exponential(v, 5)
}

fn large_exp(v: f64) -> String {
    let o = strip_decimal(&to_fixed(v, 11));
    let w = if v < 0.0 { 12 } else { 11 };
    if o.len() > w || o == "0" || o == "-0" {
        to_precision(v, 6)
    } else {
        o
    }
}

/// `SheetJS` `SSF_general`: the text a number cell shows (and the CSV writer emits) in the General
/// format.
#[must_use]
pub fn general_number(v: f64) -> String {
    if v.is_nan() {
        return "#NUM!".to_owned();
    }
    if v.is_infinite() {
        return "#DIV/0!".to_owned();
    }
    #[allow(clippy::cast_possible_truncation)]
    if v.fract() == 0.0 && v.abs() < 2_147_483_648.0 {
        // `(v | 0) === v` holds for int32 values, `-0` included.
        return format!("{}", v as i64);
    }
    let magnitude = (v.abs().ln() * std::f64::consts::LOG10_E).floor();
    #[allow(clippy::cast_possible_truncation)]
    let exponent = magnitude as i32;
    let o = if (-4..=-1).contains(&exponent) {
        to_precision(v, usize::try_from(10 + exponent).unwrap_or(10))
    } else if exponent.abs() <= 9 {
        small_exp(v)
    } else if exponent == 10 {
        to_fixed(v, 10).chars().take(12).collect()
    } else {
        large_exp(v)
    };
    strip_decimal(&normalize_exp(&o.to_uppercase()))
}

/// `Number(string)`: the `StringToNumber` grammar, `NaN` when the text is not a numeric literal.
#[must_use]
pub fn js_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    let lower = t.to_ascii_lowercase();
    for (prefix, radix) in [("0x", 16u32), ("0o", 8), ("0b", 2)] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if rest.is_empty() {
                return f64::NAN;
            }
            let mut acc = 0f64;
            for c in rest.chars() {
                let Some(d) = c.to_digit(radix) else {
                    return f64::NAN;
                };
                acc = acc * f64::from(radix) + f64::from(d);
            }
            return acc;
        }
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if decimal_literal_len(t) == Some(t.len()) {
        return t.parse::<f64>().unwrap_or(f64::NAN);
    }
    f64::NAN
}

/// Length in bytes of the longest `StrDecimalLiteral` prefix (without `Infinity`), if any.
fn decimal_literal_len(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        let save = i;
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - fs;
        if int_digits == 0 && frac_digits == 0 {
            i = save;
        }
    }
    if int_digits == 0 && frac_digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let es = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > es {
            i = j;
        }
    }
    Some(i)
}

/// `parseFloat`: the longest decimal prefix after leading white space, `NaN` when there is none.
#[must_use]
pub fn js_parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(is_js_space);
    for (prefix, value) in [
        ("Infinity", f64::INFINITY),
        ("+Infinity", f64::INFINITY),
        ("-Infinity", f64::NEG_INFINITY),
    ] {
        if t.starts_with(prefix) {
            return value;
        }
    }
    match decimal_literal_len(t) {
        Some(len) => t[..len].parse::<f64>().unwrap_or(f64::NAN),
        None => f64::NAN,
    }
}

/// `SheetJS` `fuzzynum`: `Number()` first, then thousands separators, `$`, `%` and `(negative)`.
#[must_use]
pub fn fuzzy_number(s: &str) -> f64 {
    let v = js_number(s);
    if !v.is_nan() {
        return if v.is_finite() { v } else { f64::NAN };
    }
    if !s.bytes().any(|b| b.is_ascii_digit()) {
        return v;
    }
    let mut weight = 1f64;
    let chars: Vec<char> = s.chars().collect();
    let mut cleaned = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit()
            && i + 2 < chars.len()
            && chars[i + 1] == ','
            && chars[i + 2].is_ascii_digit()
        {
            cleaned.push(chars[i]);
            cleaned.push(chars[i + 2]);
            i += 3;
        } else {
            cleaned.push(chars[i]);
            i += 1;
        }
    }
    let cleaned: String = cleaned
        .chars()
        .filter(|c| {
            if *c == '%' {
                weight *= 100.0;
            }
            *c != '$' && *c != '%'
        })
        .collect();
    let v = js_number(&cleaned);
    if !v.is_nan() {
        return v / weight;
    }
    // The first `(...)` pair with no parenthesis inside, as `/[(]([^()]*)[)]/` finds it.
    for (open, _) in cleaned.match_indices('(') {
        let rest = &cleaned[open + 1..];
        let Some(close) = rest.find(')') else { break };
        let inner = &rest[..close];
        if inner.contains('(') {
            continue;
        }
        let rebuilt = format!("{}{}{}", &cleaned[..open], inner, &rest[close + 1..]);
        let v = js_number(&rebuilt);
        if !v.is_nan() {
            return v / -weight;
        }
        break;
    }
    f64::NAN
}

/// `Math.round`: ties go toward positive infinity.
#[must_use]
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// The canonical array index a property key denotes, if it is one.
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty()
        || (key.len() > 1 && key.starts_with('0'))
        || !key.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    key.parse::<u32>().ok().filter(|n| *n != u32::MAX)
}

/// The own-property order of a JavaScript object built by repeated `object[key] = value`: a repeated
/// key overwrites in place, and array-index keys enumerate first in ascending order.
#[must_use]
pub fn order_properties<T>(entries: Vec<(String, T)>) -> Vec<(String, T)> {
    let mut unique: Vec<(String, T)> = Vec::new();
    for (k, v) in entries {
        if let Some(slot) = unique.iter_mut().find(|(key, _)| *key == k) {
            slot.1 = v;
        } else {
            unique.push((k, v));
        }
    }
    let mut indexed: Vec<(u32, (String, T))> = Vec::new();
    let mut named = Vec::new();
    for entry in unique {
        match array_index(&entry.0) {
            Some(i) => indexed.push((i, entry)),
            None => named.push(entry),
        }
    }
    indexed.sort_by_key(|(i, _)| *i);
    indexed.into_iter().map(|(_, e)| e).chain(named).collect()
}
