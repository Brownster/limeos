use super::{PlannedTimestamp, valid_time};

fn number(text: &str) -> Option<i64> {
    text.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| text.parse().ok())
        .flatten()
}
fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
fn days_before_year(year: i64) -> i64 {
    let previous = year - 1;
    previous * 365 + previous / 4 - previous / 100 + previous / 400
}

/// Explicit supported ISO calendar profile; other Python spellings need review.
pub(super) fn parse(text: &str) -> Option<PlannedTimestamp> {
    if !text.is_ascii() || !(20..=32).contains(&text.len()) {
        return None;
    }
    let bytes = text.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let year = number(&text[..4])?;
    let month = number(&text[5..7])?;
    let day = number(&text[8..10])?;
    let hour = number(&text[11..13])?;
    let minute = number(&text[14..16])?;
    let second = number(&text[17..19])?;
    let months = [
        31,
        28 + i64::from(leap(year)),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=months[(month - 1) as usize]).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut cursor = 19;
    let mut micros = 0;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if !(1..=6).contains(&(cursor - start)) {
            return None;
        }
        micros = number(&text[start..cursor])? as u32 * 10u32.pow(6 - (cursor - start) as u32);
    }
    let offset = match bytes.get(cursor) {
        Some(b'Z') if cursor + 1 == bytes.len() => 0,
        Some(sign @ (b'+' | b'-')) if cursor + 6 == bytes.len() => {
            if bytes[cursor + 3] != b':' {
                return None;
            }
            let hours = number(&text[cursor + 1..cursor + 3])?;
            let minutes = number(&text[cursor + 4..])?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            (hours * 3600 + minutes * 60) * if *sign == b'-' { -1 } else { 1 }
        }
        _ => return None,
    };
    let days = days_before_year(year) - days_before_year(1970)
        + months[..(month - 1) as usize].iter().sum::<i64>()
        + day
        - 1;
    let unix_seconds = days * 86400 + hour * 3600 + minute * 60 + second - offset;
    valid_time(unix_seconds).then(|| PlannedTimestamp {
        text: text.into(),
        unix_seconds,
        micros,
    })
}
