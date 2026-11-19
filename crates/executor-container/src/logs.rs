use limeos_domain::{CONTAINER_LOG_BYTES, ContainerLogs, Error, ErrorCode, Result};
use std::collections::VecDeque;

/// Decoded output kept while a logs response streams: its newest bytes.
const WINDOW: usize = 64 * 1024;
/// Whole-response ceiling. The Engine splits log entries at 16 KiB, so 200
/// tail entries plus framing and timestamps stay well inside it.
const MAX_RESPONSE: usize = 8 * 1024 * 1024;
const KEY_REDACTED: &str = "[private key redacted]\n";
const CREDENTIAL_REDACTED: &str = "[credential-bearing line redacted]\n";
const WITHHELD: &str = "[possible private key lines withheld]\n";

/// Demultiplexes a logs response as it arrives and keeps only the newest
/// `WINDOW` bytes of output. The Engine sends the last lines oldest first, so
/// the end is what matters; memory stays bounded however long the response.
pub(super) struct Tail {
    tty: bool,
    header: Vec<u8>,
    remaining: usize,
    window: VecDeque<u8>,
    cut: bool,
    read: usize,
}
impl Tail {
    pub(super) fn new(tty: bool) -> Self {
        Self {
            tty,
            header: Vec::with_capacity(8),
            remaining: 0,
            window: VecDeque::new(),
            cut: false,
            read: 0,
        }
    }
    /// One piece of the body, cut anywhere: frame headers may span pieces.
    pub(super) fn push(&mut self, mut data: &[u8]) -> Result<()> {
        self.read = self.read.saturating_add(data.len());
        if self.read > MAX_RESPONSE {
            return Err(Error(ErrorCode::Unavailable));
        }
        if self.tty {
            self.keep(data);
            return Ok(());
        }
        while !data.is_empty() {
            if self.remaining == 0 {
                let Some((&byte, rest)) = data.split_first() else {
                    break;
                };
                data = rest;
                self.header.push(byte);
                let Ok([stream, a, b, c, l0, l1, l2, l3]) =
                    <[u8; 8]>::try_from(self.header.as_slice())
                else {
                    continue;
                };
                if !matches!(stream, 1 | 2) || [a, b, c] != [0, 0, 0] {
                    return Err(Error(ErrorCode::Unavailable));
                }
                self.header.clear();
                self.remaining = u32::from_be_bytes([l0, l1, l2, l3]) as usize;
                continue;
            }
            let (payload, rest) = data.split_at(self.remaining.min(data.len()));
            self.keep(payload);
            self.remaining -= payload.len();
            data = rest;
        }
        Ok(())
    }
    fn keep(&mut self, bytes: &[u8]) {
        let bytes = match bytes.len().checked_sub(WINDOW) {
            Some(older) if older > 0 => {
                self.cut = true;
                bytes.get(older..).unwrap_or_default()
            }
            _ => bytes,
        };
        let overflow = (self.window.len() + bytes.len()).saturating_sub(WINDOW);
        if overflow > 0 {
            self.cut = true;
            self.window.drain(..overflow.min(self.window.len()));
        }
        self.window.extend(bytes);
    }
    /// The complete response has been read. A partial frame is malformed.
    pub(super) fn finish(self, resource: String) -> Result<ContainerLogs> {
        if !self.header.is_empty() || self.remaining != 0 {
            return Err(Error(ErrorCode::Unavailable));
        }
        let bytes = Vec::from(self.window);
        // Older output was dropped: start after the first partial line.
        let start = match bytes.iter().position(|&b| b == b'\n') {
            Some(end) if self.cut => end + 1,
            _ => 0,
        };
        Ok(decode(
            resource,
            bytes.get(start..).unwrap_or_default(),
            self.cut,
        ))
    }
}

/// The newest whole output lines that fit `CONTAINER_LOG_BYTES`, after
/// terminal controls are stripped and credentials redacted.
fn decode(resource: String, bytes: &[u8], cut: bool) -> ContainerLogs {
    let clean = strip_controls(&String::from_utf8_lossy(bytes));
    let mut total = 0;
    render(&clean, |piece| total += piece.len());
    let mut text = String::new();
    let mut truncated = cut;
    render(&clean, |piece| {
        if total > CONTAINER_LOG_BYTES {
            // Only the newest line is left and alone it is too long: keep its end.
            if total == piece.len() {
                let mut start = piece.len() - CONTAINER_LOG_BYTES;
                while !piece.is_char_boundary(start) {
                    start += 1;
                }
                text.push_str(piece.get(start..).unwrap_or_default());
            }
            total -= piece.len();
            truncated = true;
            return;
        }
        text.push_str(piece);
    });
    ContainerLogs {
        resource,
        text,
        truncated,
    }
}

fn key_begin(line: &str) -> bool {
    line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----")
}
fn key_end(line: &str) -> bool {
    line.contains("-----END") && line.contains("PRIVATE KEY-----")
}
/// A line's text without the Engine's leading timestamp (`timestamps=1`).
fn content(line: &str) -> &str {
    let line = line.trim_end_matches(['\n', '\r']);
    match line.split_once(' ') {
        Some((stamp, rest))
            if stamp.len() <= 40
                && stamp.starts_with(|c: char| c.is_ascii_digit())
                && stamp.contains('T')
                && stamp.ends_with('Z') =>
        {
            rest
        }
        _ => line,
    }
}
fn base64_token(text: &str, lengths: std::ops::RangeInclusive<usize>) -> bool {
    lengths.contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}
/// Each output piece of `clean` in order: a line as it is, or its redaction.
///
/// The output never shows where it starts: the Engine applies `tail=N` before
/// anything arrives and our window may drop more. So, whether or not this
/// reader cut anything, the start is treated as having lost its context:
/// - if the first key marker is an END line, everything before it is inside
///   a block whose BEGIN is gone and is redacted;
/// - otherwise leading lines that look like a wrapped PEM body are withheld:
///   a bare base64-alphabet token of 40 to 76 characters (PEM wraps at 64,
///   MIME at 76), then at most one shorter bare token directly after them
///   (a block's last line). The first line that is neither ends this; such
///   lines only stay hidden while they open the output, so the cost is a
///   leading bare digest or token of that length.
fn render<'a>(clean: &'a str, mut emit: impl FnMut(&'a str)) {
    let mut private_key = clean
        .split_inclusive('\n')
        .find(|line| key_begin(line) || key_end(line))
        .is_some_and(
            |line| match (line.find("-----END"), line.find("-----BEGIN")) {
                (Some(end), Some(begin)) => end < begin,
                (Some(_), None) => true,
                _ => false,
            },
        );
    if private_key {
        emit(KEY_REDACTED);
    }
    let mut leading = !private_key;
    let mut withheld = false;
    for line in clean.split_inclusive('\n') {
        if leading {
            if base64_token(content(line), 40..=76) {
                withheld = true;
                continue;
            }
            leading = false;
            if withheld {
                emit(WITHHELD);
                if base64_token(content(line), 1..=39) {
                    continue;
                }
            }
        }
        if key_begin(line) {
            if !private_key {
                emit(KEY_REDACTED);
            }
            // A block may open and close on this one line.
            private_key = !(key_end(line) && line.rfind("-----END") > line.rfind("-----BEGIN"));
            continue;
        }
        if private_key {
            if key_end(line) {
                private_key = false;
            }
            continue;
        }
        emit(if credential(line) {
            CREDENTIAL_REDACTED
        } else {
            line
        });
    }
    if leading && withheld {
        emit(WITHHELD);
    }
}
fn credential(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
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
            part.split_once("://")
                .is_some_and(|(_, rest)| rest.split('/').next().unwrap_or_default().contains('@'))
        })
        || lower
            .split(|c: char| c.is_whitespace() || "\"'=:,[]{}".contains(c))
            .any(|part| part.starts_with("sk-") && part.len() >= 16)
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
    fn decode_all(raw: &[u8], tty: bool) -> Result<ContainerLogs> {
        let mut tail = Tail::new(tty);
        tail.push(raw)?;
        tail.finish("r".into())
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
        let value = decode_all(&raw, false).unwrap();
        assert_eq!(
            value.text,
            "ready\n[credential-bearing line redacted]\n[credential-bearing line redacted]\nerror\n[credential-bearing line redacted]\n<html>literal</html>\n"
        );
        assert!(!value.truncated);
    }
    #[test]
    fn malformed_or_partial_frames_fail_closed_and_output_remains_bounded() {
        assert!(decode_all(b"invalid", false).is_err());
        assert!(decode_all(b"\x03\0\0\0\0\0\0\x01x", false).is_err());
        let mut raw = frame(1, b"partial");
        raw[7] = 100;
        assert!(decode_all(&raw, false).is_err());
        let mut tail = Tail::new(true);
        for _ in 0..MAX_RESPONSE / WINDOW {
            tail.push(&[b'z'; WINDOW]).unwrap();
        }
        assert!(tail.push(b"z").is_err());
        let large = "é".repeat(CONTAINER_LOG_BYTES);
        let value = decode_all(large.as_bytes(), true).unwrap();
        assert_eq!(value.text.len(), CONTAINER_LOG_BYTES);
        assert!(value.truncated);
    }
    #[test]
    fn frames_split_at_every_offset_resynchronise_and_memory_stays_at_the_window() {
        let mut raw = Vec::new();
        for i in 0..400 {
            raw.extend(frame(
                1 + (i % 2) as u8,
                format!("line {i:03} {}\n", "y".repeat(300)).as_bytes(),
            ));
        }
        raw.extend(frame(1, b""));
        let whole = decode_all(&raw, false).unwrap();
        for size in [1, 3, 7, 8, 9, 4096] {
            let mut tail = Tail::new(false);
            for piece in raw.chunks(size) {
                tail.push(piece).unwrap();
                assert!(tail.window.len() <= WINDOW);
            }
            let value = tail.finish("r".into()).unwrap();
            assert_eq!(value.text, whole.text, "{size}");
        }
        assert!(whole.truncated);
        assert!(whole.text.starts_with("line "));
        assert!(
            whole
                .text
                .ends_with(&format!("line 399 {}\n", "y".repeat(300)))
        );
    }
    #[test]
    fn a_window_cut_inside_a_private_key_never_shows_the_key_body() {
        let mut text = String::from("-----BEGIN PRIVATE KEY-----\n");
        while text.len() < WINDOW + 100 {
            text.push_str("MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC\n");
        }
        text.push_str("-----END PRIVATE KEY-----\nafter\n");
        let value = decode_all(text.as_bytes(), true).unwrap();
        assert_eq!(value.text, "[private key redacted]\nafter\n");
        assert!(value.truncated);
    }
    const BODY: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7VJTUt9Us8cKj";
    fn stamped(i: usize, text: &str) -> String {
        format!("2026-11-19T00:00:00.{i:09}Z {text}\n")
    }
    #[test]
    fn an_engine_tail_that_starts_inside_a_key_never_shows_the_key_body() {
        // Under the window, so nothing was cut here: the Engine's tail was.
        let raw = [
            stamped(1, BODY),
            stamped(2, "Ag=="),
            stamped(3, "-----END PRIVATE KEY-----"),
            stamped(4, "after"),
        ]
        .concat();
        let value = decode_all(&frame(1, raw.as_bytes()), false).unwrap();
        assert_eq!(value.text, format!("{KEY_REDACTED}{}", stamped(4, "after")));
        assert!(!value.truncated);
    }
    #[test]
    fn an_unfinished_key_whose_begin_was_dropped_is_withheld() {
        // Our window dropped BEGIN and logging stopped before END.
        let mut text = String::from("-----BEGIN PRIVATE KEY-----\n");
        let mut i = 0;
        while text.len() < WINDOW + 100 {
            text.push_str(&stamped(i, BODY));
            i += 1;
        }
        let value = decode_all(text.as_bytes(), true).unwrap();
        assert_eq!(value.text, WITHHELD);
        assert!(value.truncated);
        // The Engine's tail dropped BEGIN; the application then logged on.
        let raw = [
            stamped(1, BODY),
            stamped(2, BODY),
            stamped(3, "lQIDAQAB"),
            stamped(4, "listening on :8080"),
        ]
        .concat();
        let value = decode_all(raw.as_bytes(), true).unwrap();
        assert_eq!(
            value.text,
            format!("{WITHHELD}{}", stamped(4, "listening on :8080"))
        );
    }
    #[test]
    fn withholding_leading_key_like_lines_costs_ordinary_logs_only_a_leading_bare_token() {
        let ordinary = [
            stamped(1, "Ready"),
            stamped(2, "OK"),
            stamped(
                3,
                r#"10.0.0.2 - - [19/Nov/2026:00:00:01 +0000] "GET / HTTP/1.1" 200 612"#,
            ),
            stamped(4, r#"{"level":"info","msg":"started"}"#),
            stamped(5, "Starting server on port 8080"),
            stamped(6, &"0123456789abcdef".repeat(4)),
        ]
        .concat();
        let value = decode_all(ordinary.as_bytes(), true).unwrap();
        assert_eq!(value.text, ordinary);
        // The cost: a bare base64-alphabet token of wrapped-key length that
        // opens the output (here a digest) is withheld with the line after it
        // only if that one is short and bare too.
        let leading = [
            stamped(1, &"0123456789abcdef".repeat(4)),
            stamped(2, "Starting server"),
        ]
        .concat();
        let value = decode_all(leading.as_bytes(), true).unwrap();
        assert_eq!(
            value.text,
            format!("{WITHHELD}{}", stamped(2, "Starting server"))
        );
    }
    #[test]
    fn private_key_blocks_on_one_or_many_lines_redact_only_the_block() {
        let raw = b"a\n-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----\nb\nk=-----BEGIN PRIVATE KEY-----x-----END PRIVATE KEY-----\nc\n";
        let value = decode_all(raw, true).unwrap();
        assert_eq!(
            value.text,
            "a\n[private key redacted]\nb\n[private key redacted]\nc\n"
        );
        assert!(!value.truncated);
    }
}
