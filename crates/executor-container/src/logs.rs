use limeos_domain::{CONTAINER_LOG_BYTES, ContainerLogs, Error, ErrorCode, Result};

pub(super) fn decode(
    resource: String,
    raw: &[u8],
    tty: bool,
    clipped: bool,
) -> Result<ContainerLogs> {
    let mut bytes = Vec::new();
    let mut truncated = clipped;
    if tty {
        bytes.extend_from_slice(raw);
    } else {
        let mut position = 0;
        while position < raw.len() {
            let header = raw.get(position..position + 8);
            let Some(header) = header else {
                if clipped {
                    break;
                }
                return Err(Error(ErrorCode::Unavailable));
            };
            if !matches!(header[0], 1 | 2) || header[1..4] != [0, 0, 0] {
                return Err(Error(ErrorCode::Unavailable));
            }
            let length = u32::from_be_bytes(header[4..8].try_into().unwrap()) as usize;
            position += 8;
            let available = raw.len() - position;
            if length > available && !clipped {
                return Err(Error(ErrorCode::Unavailable));
            }
            let count = length.min(available);
            bytes.extend_from_slice(&raw[position..position + count]);
            position += count;
            if count < length {
                truncated = true;
                break;
            }
        }
    }
    let clean = strip_controls(&String::from_utf8_lossy(&bytes));
    let mut text = String::new();
    let mut private_key = false;
    for line in clean.split_inclusive('\n') {
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            private_key = true;
            if text.len() + 23 <= CONTAINER_LOG_BYTES {
                text.push_str("[private key redacted]\n");
            } else {
                truncated = true;
                break;
            }
            continue;
        }
        if private_key {
            if line.contains("-----END") && line.contains("PRIVATE KEY-----") {
                private_key = false;
            }
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let credential = [
            "password",
            "passwd",
            "passphrase",
            "secret",
            "api_key",
            "api-key",
            "apikey",
            "token",
            "authorization",
            "bearer ",
            "ghp_",
            "github_pat_",
            "sk-ant-",
            "sk-proj-",
            "x-amz-credential",
            "x-amz-signature",
            "x-goog-signature",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
            || lower.split_whitespace().any(|part| {
                part.split_once("://").is_some_and(|(_, rest)| {
                    rest.split('/').next().unwrap_or_default().contains('@')
                })
            })
            || lower
                .split(|c: char| c.is_whitespace() || "\"'=:,[]{}".contains(c))
                .any(|part| part.starts_with("sk-") && part.len() >= 16);
        let line = if credential {
            "[credential-bearing line redacted]\n"
        } else {
            line
        };
        let left = CONTAINER_LOG_BYTES.saturating_sub(text.len());
        if line.len() > left {
            let mut end = left;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            text.push_str(&line[..end]);
            truncated = true;
            break;
        }
        text.push_str(line);
    }
    Ok(ContainerLogs {
        resource,
        text,
        truncated,
    })
}

fn strip_controls(value: &str) -> String {
    enum State {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }
    let mut state = State::Text;
    let mut output = String::new();
    for c in value.chars() {
        state = match state {
            State::Text if c == '\u{1b}' => State::Escape,
            State::Text => {
                if (c == '\n' || c == '\t' || !c.is_control())
                    && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                {
                    output.push(c);
                }
                State::Text
            }
            State::Escape => match c {
                '[' => State::Csi,
                ']' => State::Osc,
                _ => State::Text,
            },
            State::Csi if ('@'..='~').contains(&c) => State::Text,
            State::Csi => State::Csi,
            State::Osc if c == '\u{7}' => State::Text,
            State::Osc if c == '\u{1b}' => State::OscEscape,
            State::Osc => State::Osc,
            State::OscEscape if c == '\\' => State::Text,
            State::OscEscape => State::Osc,
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(stream: u8, text: &[u8]) -> Vec<u8> {
        let mut raw = vec![stream, 0, 0, 0];
        raw.extend_from_slice(&(text.len() as u32).to_be_bytes());
        raw.extend_from_slice(text);
        raw
    }
    #[test]
    fn decode_streams_strip_terminal_controls_and_redact_credentials() {
        let mut raw = frame(
            1,
            b"ready\nAPI_KEY=private\n\x1b[31mpass\x1b[0mword=hidden\n",
        );
        raw.extend(frame(
            2,
            b"error\nAuthorization: Bearer private\n\x1b]52;c;clipboard\x07<html>literal</html>\n",
        ));
        let value = decode("resource".into(), &raw, false, false).unwrap();
        assert_eq!(
            value.text,
            "ready\n[credential-bearing line redacted]\n[credential-bearing line redacted]\nerror\n[credential-bearing line redacted]\n<html>literal</html>\n"
        );
        assert!(!value.truncated);
    }
    #[test]
    fn malformed_frames_fail_closed_and_clipping_remains_bounded() {
        assert!(decode("r".into(), b"invalid", false, false).is_err());
        let mut raw = frame(1, b"partial");
        raw[7] = 100;
        assert!(decode("r".into(), &raw, false, false).is_err());
        assert_eq!(
            decode("r".into(), &raw, false, true).unwrap().text,
            "partial"
        );
        let large = "é".repeat(CONTAINER_LOG_BYTES);
        let value = decode("r".into(), large.as_bytes(), true, false).unwrap();
        assert_eq!(value.text.len(), CONTAINER_LOG_BYTES);
        assert!(value.truncated);
    }
}
