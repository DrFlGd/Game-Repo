//! Which of two files of an Atlas pack is the newer (docs/PLAN.md, D216 and D222 revised):
//! the later `pack.updated`, then the later `exportedAt`, compared as instants, the rule
//! Atlas and Atlas Importer follow. Here so Game Repo (`gamerepo-core`'s `atlaspack`) and
//! Game Repo Go (`gamerepo-device`'s `packs`, before it replaces a pack on a device) decide
//! it the same way.

use std::cmp::Ordering;

/// Which of two files of a pack is the newer, by their descriptions' `pack.updated`, then
/// their `exportedAt` (Equal: neither is). A date that's missing or can't be read counts as
/// earlier than one that can.
pub fn newer(updated: Option<&str>, exported_at: Option<&str>, other_updated: Option<&str>, other_exported_at: Option<&str>) -> Ordering {
    later(updated, other_updated).then_with(|| later(exported_at, other_exported_at))
}

/// An ISO 8601 date and time as seconds and nanoseconds since 1970 (UTC), for comparing.
pub fn instant(s: &str) -> Option<(i64, u32)> {
    let s = s.trim();
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse().ok()) };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let (mut h, mut mi, mut sec, mut nanos, mut at) = (0, 0, 0, 0u32, 10);
    if matches!(b.get(10), Some(b'T') | Some(b't') | Some(b' ')) {
        h = num(11..13)?;
        mi = num(14..16)?;
        at = 16;
        if b.get(16) == Some(&b':') {
            sec = num(17..19)?;
            at = 19;
        }
        if b.get(at) == Some(&b'.') {
            let digits: String = s[at + 1..].chars().take_while(char::is_ascii_digit).collect();
            at += 1 + digits.len();
            nanos = format!("{:0<9}", &digits[..digits.len().min(9)]).parse().unwrap_or(0);
        }
    }
    let offset = match s.get(at..) {
        None | Some("") | Some("Z") | Some("z") => 0,
        Some(o) if (o.starts_with('+') || o.starts_with('-')) && o.len() >= 3 => {
            let oh: i64 = o.get(1..3)?.parse().ok()?;
            let om: i64 = o.get(o.len() - 2..).filter(|_| o.len() >= 5)?.parse().unwrap_or(0);
            (oh * 3600 + om * 60) * if o.starts_with('-') { -1 } else { 1 }
        }
        _ => return None,
    };
    // days from the civil date (Howard Hinnant's days_from_civil)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 + sec - offset, nanos))
}

/// Which of two dates is later; one that can't be read counts as earlier than one that can.
pub fn later(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a.and_then(instant), b.and_then(instant)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => a.unwrap_or("").cmp(b.unwrap_or("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_updated_decides_then_exported_at() {
        let (a, b) = (Some("2026-11-17T08:30:00Z"), Some("2026-11-17T09:30:00+02:00"));
        assert_eq!(later(a, b), Ordering::Greater, "08:30 UTC is after 07:30 UTC");
        let (x, y) = (Some("2026-11-18T21:04:11Z"), Some("2026-11-19T00:00:00.5Z"));
        assert_eq!(newer(a, x, a, y), Ordering::Less, "exportedAt decides a tie");
        assert_eq!(newer(a, y, a, x), Ordering::Greater);
        assert_eq!(newer(a, x, b, y), Ordering::Greater, "pack.updated first");
        assert_eq!(newer(a, x, a, x), Ordering::Equal);
        assert_eq!(later(Some("2026-11-17T08:30:00.000Z"), Some("2026-11-17T08:30:00Z")), Ordering::Equal);
        assert_eq!(later(None, a), Ordering::Less, "a date that's missing is earlier");
        assert_eq!(later(Some("not a date"), a), Ordering::Less);
    }
}
