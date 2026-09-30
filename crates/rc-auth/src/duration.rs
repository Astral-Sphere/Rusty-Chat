//! `parse_duration` — open-webui `utils/misc.py` parity.
//!
//! Grammar: `-1`/`0` → never expires (None); otherwise one or more
//! `<number><ms|s|m|h|d|w>` pairs summed together (e.g. `30d`, `1h30m`,
//! `1.5d`). Invalid strings are errors, matching the Python ValueError.

use rc_core::{Error, Result};
use std::time::Duration;

/// Parsed duration in a concrete unit; `Never` mirrors the Python `None`
/// (used by `auth.jwt_expiry = -1/0` to disable expiry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedDuration {
    Never,
    Finite(Duration),
}

/// Parses a duration string. Numbers may be fractional (`1.5d`); unknown
/// fragments between valid pairs are ignored exactly like the Python
/// `re.findall` scan.
pub fn parse_duration(s: &str) -> Result<ParsedDuration> {
    if s == "-1" || s == "0" {
        return Ok(ParsedDuration::Never);
    }

    let mut total_ms = 0f64;
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut matched = false;

    while i < bytes.len() {
        // scan a number: digits with optional fraction
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        if i == start || i >= bytes.len() {
            i += 1;
            continue;
        }
        let number: f64 = s[start..i]
            .parse()
            .map_err(|_| Error::BadRequest(format!("Invalid duration string: {s}")))?;
        // scan a unit
        let unit_len = match bytes[i] {
            b'm' if i + 1 < bytes.len() && bytes[i + 1] == b's' => 2,
            b'm' | b's' | b'h' | b'd' | b'w' => 1,
            _ => {
                i += 1;
                continue;
            }
        };
        let unit = &s[i..i + unit_len];
        i += unit_len;
        matched = true;
        let ms = match unit {
            "ms" => number,
            "s" => number * 1000.0,
            "m" => number * 60.0 * 1000.0,
            "h" => number * 3600.0 * 1000.0,
            "d" => number * 86400.0 * 1000.0,
            "w" => number * 604800.0 * 1000.0,
            _ => unreachable!("unit scan guarantees one of ms|s|m|h|d|w"),
        };
        total_ms += ms;
    }

    if !matched {
        return Err(Error::BadRequest(format!("Invalid duration string: {s}")));
    }
    Ok(ParsedDuration::Finite(Duration::from_millis(
        total_ms as u64,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ -1/0 → Never；单位逐一正确（ms/s/m/h/d/w）；组合（1h30m）、小数（1.5d）
    // ✅ 无效串报错（纯文字、缺单位、缺数字）
    // ✅ `0s` → Finite(0)（裸 "0" 才是 Never 哨兵）
    // ✅ 极大值 f64→u64 饱和不 panic
    // ✅ 对拍钉死的刻意分歧（D-016，Python misc.py parse_duration）：
    //    "-5s" → 我们 +5s（Python 为负 timedelta）；"1.2.3d" → 我们报错
    //    （Python findall 取 "2.3d"=2.3 天）；对空白片段双方都忽略
    // ⛔ 刻意不覆盖：负时长语义对齐（业务配置不使用负值，钉死即可）

    #[test]
    fn never_cases() {
        assert_eq!(parse_duration("-1").unwrap(), ParsedDuration::Never);
        assert_eq!(parse_duration("0").unwrap(), ParsedDuration::Never);
    }

    #[test]
    fn single_units() {
        assert_eq!(
            parse_duration("30s").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(30))
        );
        assert_eq!(
            parse_duration("5m").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(300))
        );
        assert_eq!(
            parse_duration("2h").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(7200))
        );
        assert_eq!(
            parse_duration("30d").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(2_592_000))
        );
        assert_eq!(
            parse_duration("1w").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(604_800))
        );
        assert_eq!(
            parse_duration("250ms").unwrap(),
            ParsedDuration::Finite(Duration::from_millis(250))
        );
    }

    #[test]
    fn compound_and_fractional() {
        assert_eq!(
            parse_duration("1h30m").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(5400))
        );
        assert_eq!(
            parse_duration("1.5d").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(129_600))
        );
    }

    #[test]
    fn invalid_inputs() {
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("30").is_err()); // number without unit
        assert!(parse_duration("d").is_err()); // unit without number
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn zero_unit_is_finite_zero_not_never() {
        // bare "0" is the Never sentinel; "0s" is a genuine zero duration
        assert_eq!(
            parse_duration("0s").unwrap(),
            ParsedDuration::Finite(Duration::ZERO)
        );
        assert_eq!(parse_duration("0").unwrap(), ParsedDuration::Never);
        assert_eq!(
            parse_duration("0ms").unwrap(),
            ParsedDuration::Finite(Duration::ZERO)
        );
    }

    #[test]
    fn negative_unit_is_absorbed_as_positive_pinned_divergence() {
        // Python misc.py findall accepts "-5s" → negative timedelta; our
        // scanner skips the '-' fragment and yields +5s. Pinned divergence
        // (D-016): no open-webui config consumes negative durations.
        assert_eq!(
            parse_duration("-5s").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(5))
        );
    }

    #[test]
    fn multi_dot_number_errors_pinned_divergence() {
        // Python findall matches "2.3d" inside "1.2.3d" → 2.3 days; our
        // strict number scan consumes "1.2.3" and fails to parse → error.
        // Pinned divergence (D-016) — inputs like this never occur in
        // open-webui config surface.
        assert!(parse_duration("1.2.3d").is_err());
    }

    #[test]
    fn huge_duration_saturates_without_panicking() {
        // 1e21 weeks ≈ 6e29 ms overflows u64 — the f64→u64 cast saturates
        assert!(matches!(
            parse_duration("999999999999999999999w").unwrap(),
            ParsedDuration::Finite(_)
        ));
    }

    #[test]
    fn whitespace_fragments_are_skipped_like_python() {
        assert_eq!(
            parse_duration("1h 30m").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(5400))
        );
    }
}
