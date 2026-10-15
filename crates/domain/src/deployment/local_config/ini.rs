use super::*;
use std::collections::BTreeSet;

fn unsupported<T>() -> Result<T> {
    Err(ConfigError::Unsupported)
}
fn section(line: &str) -> Result<(usize, &str)> {
    let open = line.bytes().take_while(|b| *b == b'[').count();
    let end = line.find(']').ok_or(ConfigError::Unsupported)?;
    let close = line[end..].bytes().take_while(|b| *b == b']').count();
    let name = line[open..end].trim();
    let tail = line[end + close..].trim();
    if open != close
        || open > MAX_DEPTH
        || name.is_empty()
        || name.len() > 128
        || name.contains(['[', '"', '\''])
        || (!tail.is_empty() && !tail.starts_with('#'))
    {
        return unsupported();
    }
    Ok((open, name))
}
fn scalar(value: &str, offset: usize) -> Result<(String, Range<usize>)> {
    let leading = value.len() - value.trim_start_matches([' ', '\t']).len();
    let value = &value[leading..];
    let start = offset + leading;
    let (decoded, end) = if value.starts_with(['\'', '"']) {
        let quote = value.as_bytes()[0] as char;
        let end = value[1..].find(quote).ok_or(ConfigError::Unsupported)? + 2;
        let tail = value[end..].trim();
        if !tail.is_empty() && !tail.starts_with('#') {
            return unsupported();
        }
        (value[1..end - 1].to_string(), end)
    } else {
        let end = value.find('#').unwrap_or(value.len());
        let raw = value[..end].trim_end_matches([' ', '\t']);
        if raw.contains([',', '"', '\'']) {
            return unsupported();
        }
        (raw.to_string(), raw.len())
    };
    if decoded.len() > MAX_PATH_BYTES {
        return Err(ConfigError::Limit);
    }
    Ok((decoded, start..start + end))
}
pub(super) fn patches(text: &str) -> Result<Layout> {
    if text.contains("\"\"\"") || text.contains("'''") {
        return unsupported();
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut settings = vec![
        Setting {
            key: "complete_dir",
            field: Field::CompleteDirectory,
            value: None,
        },
        Setting {
            key: "download_dir",
            field: Field::IncompleteDirectory,
            value: None,
        },
    ];
    let bom = usize::from(text.starts_with('\u{feff}')) * 3;
    let mut offset = bom;
    let mut misc = false;
    let mut root_misc = false;
    let mut found = false;
    let mut previous_depth = 0;
    let mut insertion = text.len();
    let mut keys = BTreeSet::new();
    for (count, full) in text[bom..].split_inclusive('\n').enumerate() {
        if count >= MAX_LINES {
            return Err(ConfigError::Limit);
        }
        let line = full.strip_suffix('\n').unwrap_or(full);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.contains('\r') {
            return unsupported();
        }
        let trimmed = line.trim_matches([' ', '\t']);
        if trimmed.starts_with('[') {
            let (depth, name) = section(trimmed)?;
            if depth > previous_depth + 1 {
                return unsupported();
            }
            previous_depth = depth;
            if depth == 1 {
                root_misc = name == "misc";
            } else if depth == 2
                && root_misc
                && settings.iter().any(|s| s.key.eq_ignore_ascii_case(name))
            {
                return unsupported();
            }
            if misc {
                insertion = offset;
            }
            misc = depth == 1 && name == "misc";
            if depth == 1 && name.eq_ignore_ascii_case("misc") {
                if found || name != "misc" {
                    return unsupported();
                }
                found = true;
            }
        } else if previous_depth == 0 && !trimmed.is_empty() && !trimmed.starts_with('#') {
            let (key, _) = trimmed.split_once('=').ok_or(ConfigError::Unsupported)?;
            let key = key.trim();
            if key.is_empty() || key.contains(['\'', '"']) || key.eq_ignore_ascii_case("misc") {
                return unsupported();
            }
        } else if misc && !trimmed.is_empty() && !trimmed.starts_with('#') {
            let equal = line.find('=').ok_or(ConfigError::Unsupported)?;
            let key = line[..equal].trim_matches([' ', '\t']);
            if key.len() > 128 || keys.len() >= MAX_FIELDS {
                return Err(ConfigError::Limit);
            }
            if key.contains(['\'', '"']) || !keys.insert(key.to_string()) {
                return unsupported();
            }
            if let Some(setting) = settings
                .iter_mut()
                .find(|s| s.key.eq_ignore_ascii_case(key))
            {
                if setting.key != key {
                    return unsupported();
                }
                setting.value = Some(scalar(&line[equal + 1..], offset + equal + 1)?);
            }
        }
        offset += full.len();
    }
    let needs_newline = insertion > bom && !text[..insertion].ends_with('\n');
    let prefix = format!(
        "{}{}",
        if needs_newline { newline } else { "" },
        if found {
            String::new()
        } else {
            format!("[misc]{newline}")
        }
    );
    Ok(Layout {
        settings,
        insertion,
        prefix,
        suffix: newline.into(),
        newline,
        fields: keys.len(),
        json: false,
    })
}
