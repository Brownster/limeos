use super::*;
use serde::de::IgnoredAny;
use std::collections::BTreeSet;

fn unsupported<T>() -> Result<T> {
    Err(ConfigError::Unsupported)
}
fn whitespace(b: &[u8], at: &mut usize) {
    while b.get(*at).is_some_and(u8::is_ascii_whitespace) {
        *at += 1;
    }
}
fn string_end(b: &[u8], start: usize) -> Result<usize> {
    let mut i = start + 1;
    while let Some(byte) = b.get(i) {
        match byte {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    unsupported()
}
fn value_end(b: &[u8], start: usize) -> Result<usize> {
    if b[start] == b'"' {
        return string_end(b, start);
    }
    let mut depth = 0;
    let mut i = start;
    while let Some(byte) = b.get(i) {
        match byte {
            b'"' => {
                i = string_end(b, i)?;
                continue;
            }
            b'{' | b'[' => {
                depth += 1;
                if depth + 1 > MAX_DEPTH {
                    return Err(ConfigError::Limit);
                }
            }
            b'}' | b']' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
            b',' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    while i > start && b[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    Ok(i)
}
pub(super) fn patches(bytes: &[u8]) -> Result<Layout> {
    serde_json::from_slice::<IgnoredAny>(bytes).map_err(|_| ConfigError::Unsupported)?;
    let mut at = 0;
    whitespace(bytes, &mut at);
    if bytes.get(at) != Some(&b'{') {
        return unsupported();
    }
    at += 1;
    let mut seen = BTreeSet::new();
    let mut insertion = at;
    let mut settings = vec![
        Setting {
            key: "download-dir",
            field: Field::CompleteDirectory,
            value: None,
        },
        Setting {
            key: "incomplete-dir",
            field: Field::IncompleteDirectory,
            value: None,
        },
        Setting {
            key: "incomplete-dir-enabled",
            field: Field::IncompleteEnabled,
            value: None,
        },
    ];
    loop {
        whitespace(bytes, &mut at);
        if bytes.get(at) == Some(&b'}') {
            break;
        }
        if seen.len() >= MAX_FIELDS {
            return Err(ConfigError::Limit);
        }
        let end = string_end(bytes, at)?;
        let key: String =
            serde_json::from_slice(&bytes[at..end]).map_err(|_| ConfigError::Unsupported)?;
        if key.len() > 128 {
            return Err(ConfigError::Limit);
        }
        if ["download_dir", "incomplete_dir", "incomplete_dir_enabled"].contains(&key.as_str())
            || !seen.insert(key.clone())
        {
            return unsupported();
        }
        at = end;
        whitespace(bytes, &mut at);
        at += 1;
        whitespace(bytes, &mut at);
        let start = at;
        at = value_end(bytes, start)?;
        insertion = at;
        if let Some(setting) = settings.iter_mut().find(|s| s.key == key) {
            let value = if setting.field == Field::IncompleteEnabled {
                serde_json::from_slice::<bool>(&bytes[start..at])
                    .map_err(|_| ConfigError::Unsupported)?
                    .to_string()
            } else {
                serde_json::from_slice::<String>(&bytes[start..at])
                    .map_err(|_| ConfigError::Unsupported)?
            };
            if value.len() > MAX_PATH_BYTES || value.chars().any(char::is_control) {
                return unsupported();
            }
            setting.value = Some((value, start..at));
        }
        whitespace(bytes, &mut at);
        if bytes.get(at) == Some(&b',') {
            at += 1;
        }
    }
    let newline = if bytes.windows(2).any(|b| b == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    Ok(Layout {
        settings,
        insertion,
        prefix: format!("{}{newline}", if seen.is_empty() { "" } else { "," }),
        suffix: String::new(),
        newline,
        fields: seen.len(),
        json: true,
    })
}
