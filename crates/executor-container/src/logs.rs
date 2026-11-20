//! Bounded container log reads: the newest whole lines, terminal controls
//! stripped, private keys and credential-bearing lines redacted.
//!
//! Redaction is line based. A line is credential-bearing by structure, not
//! because a word such as "token" appears in prose:
//! - a sensitive key (a key one of whose components is, or for some words
//!   ends with, a credential word: `password`, `db_password`, `accessToken`,
//!   `X-Api-Key`, `Cookie`, `session`) followed, after an optional closing
//!   quote or bracket (also backslash-escaped, as in JSON inside a JSON
//!   string), by a non-empty value through one of these separators: `=`,
//!   `=>`, `:` or `:=` with optional spaces (`password=x`, `"api_key": "x"`,
//!   `\"token\":\"x\"`, `user[password]=x`); spaces alone, when the value is
//!   quoted (`PASSWORD 'x'`) or the key is a command-line option
//!   (`--password x`); or a `"value":` sibling after a quoted key
//!   (`{"name":"DB_PASSWORD","value":"x"}`). A quoted value is non-empty
//!   when anything but its own closing quote follows the opening one, even a
//!   space or a delimiter (`password=";a&b"`);
//! - a value of a known secret shape anywhere in the line, after any amount
//!   of whitespace: a bearer credential, a JWT, cloud and service key
//!   prefixes, URL userinfo.
//!
//! A word that only names a credential (`password reset`, `the token bucket`,
//! `processed 4096 tokens`) is not redacted. Known limits, besides the ones
//! below: a secret on the line after its key (YAML block style `password: |`,
//! or `- name: DB_PASSWORD` then `value: x`); a key written with look-alike
//! Unicode letters; a value wrapped in base64 or another encoding (a basic
//! auth header is caught by its key, not its value); a secret stated in
//! prose (`the password is hunter2`) or passed positionally
//! (`htpasswd -b file bob hunter2`, `mysql -phunter2`).
//!
//! Key blocks with their BEGIN/END markers are
//! reliable; a key body without markers (its BEGIN dropped by the Engine's
//! tail or our window, its END never written) is caught only best effort, by
//! withholding leading lines that look like one. A known limit: the Engine
//! splits a log entry at 16 KiB, so a tail can start on a later piece of a
//! long line whose `password=` style marker was in an earlier piece; that
//! fragment carries no marker and cannot be recognised.
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
const LONG_LINE: &str = "[line over the size limit omitted]\n";

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
        // Older output was dropped: never show a line whose start is gone,
        // since its redaction marker may have been in the part dropped.
        let start = match bytes.iter().position(|&b| b == b'\n') {
            Some(end) if self.cut => end + 1,
            None if self.cut => bytes.len(),
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
/// terminal controls are stripped and credentials redacted. No line is ever
/// returned in part: one too long alone is replaced by a marker.
fn decode(resource: String, bytes: &[u8], cut: bool) -> ContainerLogs {
    let clean = strip_controls(&String::from_utf8_lossy(bytes));
    let mut total = 0;
    let omitted = render(&clean, |piece| total += piece.len());
    let mut text = String::new();
    // An omitted line is output that was not returned.
    let mut truncated = cut || omitted;
    render(&clean, |piece| {
        if total > CONTAINER_LOG_BYTES {
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
/// A line's shape after the timestamp, when its final whitespace-separated
/// token uses only the base64 alphabet (with `=` padding): how many tokens
/// it has and the final token's length.
fn base64_tail(line: &str) -> Option<(usize, usize)> {
    let mut tokens = content(line).split_whitespace();
    let token = tokens.next_back()?;
    let base64 = token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    base64.then(|| (tokens.count() + 1, token.len()))
}
/// Each output piece of `clean` in order: a line as it is, or its redaction.
/// Returns whether a line was omitted for being too long.
///
/// The output never shows where it starts: the Engine applies `tail=N` before
/// anything arrives and our window may drop more. So, whether or not this
/// reader cut anything, the start is treated as having lost its context:
/// - if the first key marker is an END line, everything before it is inside
///   a block whose BEGIN is gone and is redacted;
/// - otherwise leading lines that look like a key body are withheld: a line
///   whose final token is 16 or more base64-alphabet characters, whatever
///   precedes it (a prefix, any wrapping width), then at most one line right
///   after that run whose final token is a shorter base64-only one and which
///   has as many tokens as the run's last line (a key's last line, written
///   the same way even if a prefix such as a clock varies per line). The
///   first leading line that matches neither ends this. It only ever
///   applies to the leading run of a read, so the cost is a few leading
///   lines that end in a digest, an id or a long single word, plus an
///   ordinary line right after them with the same token count that ends in
///   a base64-alphabet word.
fn render<'a>(clean: &'a str, mut emit: impl FnMut(&'a str)) -> bool {
    let mut omitted = false;
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
    // The token count of the last withheld line, once any is withheld.
    let mut withheld: Option<usize> = None;
    for line in clean.split_inclusive('\n') {
        if leading {
            let tail = base64_tail(line);
            if let Some((tokens, 16..)) = tail {
                withheld = Some(tokens);
                continue;
            }
            leading = false;
            if let Some(run) = withheld {
                emit(WITHHELD);
                if tail.is_some_and(|(tokens, _)| tokens == run) {
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
        emit(if line.len() > CONTAINER_LOG_BYTES {
            omitted = true;
            LONG_LINE
        } else if credential(line) {
            CREDENTIAL_REDACTED
        } else {
            line
        });
    }
    if leading && withheld.is_some() {
        emit(WITHHELD);
    }
    omitted
}
/// Whether a line carries a credential, judged by structure rather than by
/// words in prose: a sensitive key given a value, or a value with a known
/// secret shape. Each scan reads the line's bytes once or a bounded number
/// of bytes per position, so the cost is linear in the line.
fn credential(line: &str) -> bool {
    let bytes = line.as_bytes();
    keyed_secret(bytes)
        || (0..bytes.len()).any(|at| secret_shape(bytes, at))
        || line.split_whitespace().any(|part| {
            part.split_once("://")
                .is_some_and(|(_, rest)| rest.split('/').next().unwrap_or_default().contains('@'))
        })
}
fn alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}
fn key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}
/// A sensitive key name followed by a separator and a value: `key=value`,
/// `key: value`, `"key":"value"`, `key = 'value'`, a header, a query
/// parameter, escaped JSON, `--key value`, `KEY 'value'` and a
/// `"name":"KEY","value":...` pair. A key is a maximal run of letters,
/// digits, `_`, `-` and `.`; the rule is in the module documentation.
fn keyed_secret(bytes: &[u8]) -> bool {
    let mut at = 0;
    while at < bytes.len() {
        let end = bytes
            .get(at..)
            .and_then(|rest| rest.iter().position(|&b| !key_byte(b)))
            .map_or(bytes.len(), |len| at + len);
        if end == at {
            at += 1;
            continue;
        }
        let key = bytes.get(at..end).unwrap_or_default();
        if sensitive_key(key) && has_value(key, bytes.get(end..).unwrap_or_default(), true) {
            return true;
        }
        at = end;
    }
    false
}
fn quote(b: u8) -> bool {
    matches!(b, b'"' | b'\'' | b'`')
}
/// The count of spaces and tabs at the start of `rest`.
fn spaces(rest: &[u8]) -> usize {
    rest.iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t'))
        .count()
}
/// What follows a sensitive `key` gives it a value: after at most four
/// closing bytes (a quote, an escaped quote `\"` or `\\\"`, a bracket `]`)
/// and optional spaces, either `=`, `=>`, `:` or `:=` and then a value; or,
/// with `sibling` and a quoted key, `,` and a `"value":` key and its value;
/// or spaces alone and then a quoted value, or any value when the key is a
/// command-line option (`--password x`). A quoted value counts unless the
/// same quote, at the same escape depth, closes it at once; an unquoted one
/// unless it starts with a space or a delimiter. Every scan moves forward
/// over bytes it reads once, except the spaces after the key, read at most
/// twice, so the cost stays linear in the line.
fn has_value(key: &[u8], rest: &[u8], sibling: bool) -> bool {
    let closing = rest
        .iter()
        .take(4)
        .take_while(|&&b| b == b'\\' || b == b']' || quote(b))
        .count();
    let quoted = rest
        .get(..closing)
        .is_some_and(|c| c.iter().any(|&b| quote(b)));
    let rest = rest.get(closing..).unwrap_or_default();
    let gap = spaces(rest);
    let after = rest.get(gap..).unwrap_or_default();
    let separator = match after {
        [b'=', b'>', ..] | [b':', b'=', ..] => 2,
        [b'=' | b':', ..] => 1,
        [b',', tail @ ..] if sibling && quoted => return value_sibling(tail),
        // Spaces alone: a quoted value, or an option's value.
        _ if gap > 0 => return value(after, !key.starts_with(b"-")),
        _ => return false,
    };
    let after = after.get(separator..).unwrap_or_default();
    value(after.get(spaces(after)..).unwrap_or_default(), false)
}
/// After a key's `,`: a quoted `value` key (any case, escapes allowed) that
/// itself has a value, as in `{"name":"DB_PASSWORD","value":"x"}`.
fn value_sibling(rest: &[u8]) -> bool {
    let rest = rest.get(spaces(rest)..).unwrap_or_default();
    let escapes = rest.iter().take(4).take_while(|&&b| b == b'\\').count();
    if !rest.get(escapes).copied().is_some_and(quote) {
        return false;
    }
    let rest = rest.get(escapes + 1..).unwrap_or_default();
    let word = rest.iter().take_while(|&&b| key_byte(b)).count();
    rest.get(..word)
        .is_some_and(|w| w.eq_ignore_ascii_case(b"value"))
        && has_value(b"value", rest.get(word..).unwrap_or_default(), false)
}
/// A non-empty value starts `rest`; with `quoted_only`, only a quoted one.
fn value(rest: &[u8], quoted_only: bool) -> bool {
    let escapes = rest.iter().take(4).take_while(|&&b| b == b'\\').count();
    match rest.get(escapes).copied() {
        Some(q) if quote(q) && escapes < 4 => {
            let inner = rest.get(escapes + 1..).unwrap_or_default();
            let closes = inner
                .get(..escapes)
                .is_some_and(|e| e.iter().all(|&b| b == b'\\'))
                && inner.get(escapes) == Some(&q);
            !inner.is_empty() && !closes && !matches!(inner.first(), Some(b'\n' | b'\r'))
        }
        _ if quoted_only => false,
        _ => rest.first().is_some_and(|&b| {
            !b.is_ascii_whitespace() && !matches!(b, b',' | b'}' | b']' | b';' | b'&')
        }),
    }
}
/// Words that make a key component sensitive when it ends with one.
const KEY_SUFFIXES: &[&[u8]] = &[
    b"password",
    b"passwd",
    b"passphrase",
    b"secret",
    b"token",
    b"apikey",
    b"privatekey",
    b"credential",
    b"credentials",
    b"cookie",
    b"signature",
];
/// Words that make a key component sensitive only when it is exactly one.
const KEY_WORDS: &[&[u8]] = &[b"session", b"sessionid", b"authorization", b"auth"];
/// A key whose name has a sensitive component. Components are split at `_`,
/// `-`, `.` and a lower-case letter or digit followed by an upper-case one
/// (`accessToken`); `api` or `private` followed by `key` counts as one.
fn sensitive_key(key: &[u8]) -> bool {
    let mut previous = Vec::new();
    for word in key.split(|&b| matches!(b, b'_' | b'-' | b'.')) {
        let mut start = 0;
        for at in 1..=word.len() {
            let camel = matches!(
                (word.get(at - 1), word.get(at)),
                (Some(a), Some(b))
                    if (a.is_ascii_lowercase() || a.is_ascii_digit()) && b.is_ascii_uppercase()
            );
            if at < word.len() && !camel {
                continue;
            }
            let part = word.get(start..at).unwrap_or_default().to_ascii_lowercase();
            start = at;
            if KEY_SUFFIXES.iter().any(|suffix| part.ends_with(suffix))
                || KEY_WORDS.contains(&part.as_slice())
                || (part == b"key" && matches!(previous.as_slice(), b"api" | b"private"))
            {
                return true;
            }
            previous = part;
        }
    }
    false
}
/// Whether a known secret value starts at `at`. Every check reads at most a
/// fixed number of bytes, except the JWT scan, which starts only where the
/// previous byte cannot belong to a JWT and so never rereads one.
fn secret_shape(bytes: &[u8], at: usize) -> bool {
    let previous = at.checked_sub(1).and_then(|p| bytes.get(p)).copied();
    if previous.is_some_and(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let rest = bytes.get(at..).unwrap_or_default();
    let run = |from: usize, len: usize, allowed: fn(u8) -> bool| {
        rest.get(from..from + len)
            .is_some_and(|run| run.iter().all(|&b| allowed(b)))
    };
    let after = |len: usize| rest.get(len).is_none_or(|b| !b.is_ascii_alphanumeric());
    // AWS access key id: AKIA or ASIA and 16 upper-case letters or digits.
    if (rest.starts_with(b"AKIA") || rest.starts_with(b"ASIA"))
        && run(4, 16, |b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && after(20)
    {
        return true;
    }
    // GitHub tokens.
    if [b"ghp_", b"gho_", b"ghs_", b"ghu_", b"ghr_"]
        .iter()
        .any(|prefix| rest.starts_with(*prefix))
        && run(4, 20, alnum)
    {
        return true;
    }
    if rest.starts_with(b"github_pat_") && run(11, 20, |b| alnum(b) || b == b'_') {
        return true;
    }
    // Slack tokens: xoxb-, xoxa-, xoxp-, xoxr-, xoxs-.
    if rest.starts_with(b"xox")
        && rest.get(3).is_some_and(|b| b"baprs".contains(b))
        && rest.get(4) == Some(&b'-')
        && run(5, 10, |b| alnum(b) || b == b'-')
    {
        return true;
    }
    // API keys in the sk- form (OpenAI, Anthropic): 20 or more key bytes
    // with a digit among the first 64, so `sk-learn-compatible` survives.
    if rest.starts_with(b"sk-") {
        let key = rest
            .get(3..)
            .unwrap_or_default()
            .iter()
            .take(64)
            .take_while(|&&b| alnum(b) || matches!(b, b'_' | b'-'));
        let (len, digit) = key.fold((0, false), |(len, digit), b| {
            (len + 1, digit || b.is_ascii_digit())
        });
        if len >= 20 && digit {
            return true;
        }
    }
    // Bearer credentials: the scheme, any spaces, then 8 or more token68
    // bytes. Each run of spaces follows one scheme word, so it is read once.
    if rest
        .get(..6)
        .is_some_and(|word| word.eq_ignore_ascii_case(b"bearer"))
    {
        let spaces = spaces(rest.get(6..).unwrap_or_default());
        let token68 =
            |b: u8| alnum(b) || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=');
        if spaces > 0 && run(6 + spaces, 8, token68) {
            return true;
        }
    }
    // JWT: base64url header starting `eyJ`, a dot, a non-empty payload and a
    // second dot (the signature may be empty).
    let jwt = |b: u8| alnum(b) || matches!(b, b'_' | b'-');
    if rest.starts_with(b"eyJ") && !previous.is_some_and(|b| jwt(b) || b == b'.') {
        let header = rest.iter().take_while(|&&b| jwt(b)).count();
        let payload = rest
            .get(header + 1..)
            .unwrap_or_default()
            .iter()
            .take_while(|&&b| jwt(b))
            .count();
        if rest.get(header) == Some(&b'.')
            && payload > 0
            && rest.get(header + 1 + payload) == Some(&b'.')
        {
            return true;
        }
    }
    false
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
        assert_eq!(value.text, LONG_LINE);
        assert!(value.truncated);
    }
    #[test]
    fn frames_split_at_every_offset_resynchronise_and_memory_stays_at_the_window() {
        let mut raw = Vec::new();
        for i in 0..400 {
            raw.extend(frame(
                1 + (i % 2) as u8,
                format!("line {i:03} {}\n", "yy ".repeat(100)).as_bytes(),
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
                .ends_with(&format!("line 399 {}\n", "yy ".repeat(100)))
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
    fn leading_lines_with_short_or_non_base64_endings_are_kept() {
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
    }
    /// A key body without its markers, wrapped at `width` and ending in a
    /// short final line, each line written through `prefix`.
    fn body(width: usize, prefix: &str) -> Vec<String> {
        let text = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7+/Vz".repeat(8);
        let mut lines: Vec<_> = text
            .as_bytes()
            .chunks(width)
            .map(|c| format!("{prefix}{}", std::str::from_utf8(c).unwrap()))
            .collect();
        lines.push(format!("{prefix}Ag=="));
        lines
    }
    #[test]
    fn leading_key_bodies_are_withheld_whatever_the_wrapping_or_prefix() {
        for width in [32, 64, 76, 80] {
            for prefix in ["", "INFO ", "app[1]: key "] {
                let mut lines = body(width, prefix);
                lines.push("listening on :8080".into());
                let raw: String = lines
                    .iter()
                    .enumerate()
                    .map(|(i, line)| stamped(i, line))
                    .collect();
                let value = decode_all(raw.as_bytes(), true).unwrap();
                assert_eq!(
                    value.text,
                    format!(
                        "{WITHHELD}{}",
                        stamped(lines.len() - 1, "listening on :8080")
                    ),
                    "{width} {prefix:?}"
                );
            }
        }
    }
    #[test]
    fn a_short_last_line_after_a_prefix_that_varies_per_line_is_withheld() {
        let mut lines: Vec<_> = body(64, "")
            .iter()
            .enumerate()
            .map(|(i, line)| format!("INFO [00:00:{i:02}] {line}"))
            .collect();
        lines.pop();
        lines.push("INFO [00:00:59] AQIDBAUGBwg=".into());
        lines.push("listening on :8080".into());
        let raw: String = lines
            .iter()
            .enumerate()
            .map(|(i, line)| stamped(i, line))
            .collect();
        let value = decode_all(raw.as_bytes(), true).unwrap();
        assert_eq!(
            value.text,
            format!(
                "{WITHHELD}{}",
                stamped(lines.len() - 1, "listening on :8080")
            )
        );
    }
    #[test]
    fn a_lone_short_last_line_of_a_leading_key_body_is_withheld() {
        let raw: String = body(64, "")
            .iter()
            .enumerate()
            .map(|(i, line)| stamped(i, line))
            .collect();
        let value = decode_all(raw.as_bytes(), true).unwrap();
        assert_eq!(value.text, WITHHELD);
    }
    #[test]
    fn ordinary_leading_lines_survive_and_only_a_leading_digest_line_is_the_cost() {
        let ordinary = [
            stamped(1, "Starting server on port 8080"),
            stamped(2, "GET /health 200"),
            stamped(3, r#"{"level":"info","msg":"ready"}"#),
            stamped(4, "connected to postgres://db:5432/app"),
            stamped(5, "OK"),
        ]
        .concat();
        let value = decode_all(ordinary.as_bytes(), true).unwrap();
        assert_eq!(value.text, ordinary);
        // The recorded cost: a leading line ending in a digest or id of 16 or
        // more base64-alphabet characters is withheld. The next line goes too
        // only if it could be that key's last line: a short base64-only final
        // token on a line with the same number of tokens.
        let digest = "0123456789abcdef".repeat(4);
        let raw = [
            stamped(1, &format!("pulled image {digest}")),
            stamped(2, "Starting server on port 8080"),
            stamped(3, "listening on :8080"),
        ]
        .concat();
        let value = decode_all(raw.as_bytes(), true).unwrap();
        assert_eq!(
            value.text,
            format!(
                "{WITHHELD}{}{}",
                stamped(2, "Starting server on port 8080"),
                stamped(3, "listening on :8080")
            )
        );
        let raw = [
            stamped(1, &format!("pulled image {digest}")),
            stamped(2, "pulled image abc"),
            stamped(3, "listening on :8080"),
        ]
        .concat();
        let value = decode_all(raw.as_bytes(), true).unwrap();
        assert_eq!(
            value.text,
            format!("{WITHHELD}{}", stamped(3, "listening on :8080"))
        );
    }
    #[test]
    fn an_uncut_read_with_one_over_long_line_is_truncated() {
        let raw = format!("before\n{}\nafter\n", "x".repeat(CONTAINER_LOG_BYTES + 1));
        let value = decode_all(&frame(1, raw.as_bytes()), false).unwrap();
        assert_eq!(value.text, format!("before\n{LONG_LINE}after\n"));
        assert!(value.truncated);
    }
    /// `line` after an ordinary leading line, so the leading key-body rule
    /// never applies: what the credential filter alone makes of it.
    fn filtered(line: &str) -> String {
        let raw = format!("start\n{line}\n");
        let value = decode_all(raw.as_bytes(), true).unwrap();
        value.text.strip_prefix("start\n").unwrap().to_owned()
    }
    const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    /// Secrets recognised by shape alone, wherever they appear.
    const SHAPES: &[&str] = &[
        "Bearer abcDEF123456",
        "bearer abc.DEF-123_456~+/==",
        "BEARER Zm9vYmFyYmF6cXV4",
        JWT,
        "eyJhbGciOiJub25lIn0.eyJzdWIiOiIxIn0.",
        "AKIAIOSFODNN7EXAMPLE",
        "ASIAY34FZKBOKMUTVV7A",
        "ghp_16C7e42F292c6912E7710c838347Ae178B4a",
        "gho_16C7e42F292c6912E7710c838347Ae178B4a",
        "ghs_16C7e42F292c6912E7710c838347Ae178B4a",
        "github_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyz",
        "xoxb-123456789012-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx",
        "xoxp-123456789012-123456789012-abcdef0123456789",
        "xoxa-2-123456789012-abcdef",
        "sk-ant-api03-abcdefghijklmnop0123456789",
        "sk-proj-abcdefghijklmnopqrstuvwxyz012345",
        "https://user:hunter2@example.com/repo.git",
        "postgres://app:s3cret@db:5432/app",
        "https://x-access-token:abc123@github.com/o/r",
    ];
    /// Surroundings a shape is redacted in.
    const CONTEXTS: &[&str] = &[
        "{}",
        "INFO using {} for upstream",
        "\"{}\"",
        "header={}",
        r#"{"msg":"auth","value":"{}","n":1}"#,
        "GET /cb?q=1&v={}&next=/ HTTP/1.1",
        "(credential {})",
        "got {}.",
        "2026-11-19T00:00:00.000000001Z {}",
    ];
    #[test]
    fn secret_shapes_are_redacted_in_every_context() {
        for shape in SHAPES {
            for context in CONTEXTS {
                let line = context.replace("{}", shape);
                assert_eq!(filtered(&line), CREDENTIAL_REDACTED, "{line:?}");
            }
        }
    }
    /// Key names whose value is a secret, written as applications do.
    const KEYS: &[&str] = &[
        "token",
        "access_token",
        "refresh-token",
        "X-Auth-Token",
        "accessToken",
        "csrftoken",
        "secret",
        "client_secret",
        "AWS_SECRET_ACCESS_KEY",
        "password",
        "db_password",
        "PGPASSWORD",
        "MYSQL_ROOT_PASSWORD",
        "passwd",
        "passphrase",
        "api_key",
        "api-key",
        "apikey",
        "X-Api-Key",
        "apiKey",
        "APIKey",
        "authorization",
        "Authorization",
        "cookie",
        "Cookie",
        "set-cookie",
        "Set-Cookie",
        "session",
        "session_id",
        "sessionId",
        "SESSION",
        "private_key",
        "credentials",
        "X-Amz-Credential",
        "X-Amz-Signature",
        "X-Goog-Signature",
        "--password",
        "config.auth.token",
    ];
    /// The ways a key and its value are written.
    const FORMS: &[&str] = &[
        "{}=v4lue",
        "{}: v4lue",
        "{} = \"v4lue\"",
        r#"{"user":"bob","{}":"v4lue"}"#,
        r#"{"{}" : "v4lue"}"#,
        "'{}': 'v4lue'",
        "level=info {}=v4lue user=bob",
        "GET /login?user=bob&{}=v4lue HTTP/1.1",
        "{}:v4lue; Path=/",
        "export {}=v4lue",
    ];
    #[test]
    fn sensitive_keys_are_redacted_in_every_form() {
        for key in KEYS {
            for form in FORMS {
                let line = form.replace("{}", key);
                assert_eq!(filtered(&line), CREDENTIAL_REDACTED, "{line:?}");
            }
        }
    }
    #[test]
    fn ordinary_lines_that_only_mention_a_sensitive_word_survive() {
        let digest = "a3ed95caeb02ffe68cdd9fd84406680ae93d633cb16422d00e8a7c22955b46d4";
        for line in [
            "processed 4096 tokens".to_owned(),
            "session started".to_owned(),
            "secretary".to_owned(),
            "the token bucket refilled".to_owned(),
            "id=550e8400-e29b-41d4-a716-446655440000 ready".to_owned(),
            format!("pulling docker.io/library/nginx@sha256:{digest}"),
            format!("image=nginx@sha256:{digest}"),
            "password reset email sent to user".to_owned(),
            "tokenizer loaded".to_owned(),
            "max_tokens=4096 prompt_tokens: 12".to_owned(),
            r#"{"usage":{"prompt_tokens":12,"completion_tokens":40}}"#.to_owned(),
            r#"{"event":"token_refreshed","user":"bob"}"#.to_owned(),
            r#"{"type":"session","id":5}"#.to_owned(),
            "msg=\"login failed for bob: password mismatch\"".to_owned(),
            "authorization failed for user bob".to_owned(),
            "the bearer of bad news".to_owned(),
            "Bearer token required".to_owned(),
            "secrets are mounted read-only".to_owned(),
            "active_sessions=5".to_owned(),
            "connected to postgres://db:5432/app".to_owned(),
            "risk-free reconciliation done in 12ms".to_owned(),
            "skipped 3 jobs".to_owned(),
            "2026-11-19T00:00:00.000000001Z GET /health 200".to_owned(),
            "Ünïcödé session 😀 started".to_owned(),
        ] {
            assert_eq!(filtered(&line), format!("{line}\n"), "{line:?}");
        }
    }
    #[test]
    fn redaction_never_panics_and_stays_bounded_on_arbitrary_input() {
        let pieces: &[&str] = &[
            "eyJ",
            ".",
            "AKIA",
            "ASIA",
            "ghp_",
            "github_pat_",
            "xoxb-",
            "sk-",
            "Bearer ",
            "://",
            "@",
            ":",
            "=",
            "\"",
            "'",
            "token",
            "Cookie",
            "api",
            "_",
            "-",
            "key",
            " ",
            "\t",
            "\n",
            "é",
            "😀",
            "\u{1b}[",
            "A1",
            "zz",
            "-----BEGIN",
            "PRIVATE KEY-----",
            "-----END",
            "&",
            ";",
            "\\",
            "]",
            ",",
            "value",
            "--",
            "password",
            "`",
        ];
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        for _ in 0..2000 {
            let mut text = String::new();
            for _ in 0..next(80) {
                text.push_str(pieces[next(pieces.len())]);
            }
            let value = decode_all(text.as_bytes(), true).unwrap();
            assert!(value.text.len() <= CONTAINER_LOG_BYTES);
            let mut bytes = text.into_bytes();
            if !bytes.is_empty() {
                let at = next(bytes.len());
                bytes[at] = 0xff;
            }
            decode_all(&bytes, true).unwrap();
        }
        // Long runs of near-matches finish and are judged once each.
        for unit in [
            "eyJ_",
            "eyJ.",
            "sk-",
            "ghp_",
            "AKIA",
            "a=",
            "token",
            "bearer ",
            "x://",
            "token ",
            "--token ",
            "bearer      ",
            "\"token\",",
            "\\\"token\\\":",
            "[token]",
        ] {
            let line = unit.repeat(CONTAINER_LOG_BYTES / unit.len() - 1);
            decode_all(line.as_bytes(), true).unwrap();
        }
    }
    /// Lines the reviewers found the structural rule emitting while the
    /// substring rule it replaced redacted them.
    const REPRODUCTIONS: &[&str] = &[
        r#""body":"{\"user\":\"bob\",\"password\":\"hunter2\"}""#,
        r#"{"level":"info","body":"{\"user\":\"bob\",\"password\":\"hunter2\"}"}"#,
        r#"{"log":"{\\\"token\\\":\\\"s3cr3t\\\"}"}"#,
        "ALTER ROLE app PASSWORD 'x'",
        "--password x",
        r#"{"name":"DB_PASSWORD","value":"x"}"#,
        "user[password]=x",
        r#"password=";a&b""#,
        r#"{"password":" hello"}"#,
        "Authorization: Bearer                     abcDEF123456",
        "token=Bearer                                      abcDEF123456",
        "Bearer                                            abcDEF123456",
    ];
    #[test]
    fn reported_misses_are_redacted() {
        for line in REPRODUCTIONS {
            assert_eq!(filtered(line), CREDENTIAL_REDACTED, "{line:?}");
        }
    }
    /// The decision of the substring rule this module replaced (as of
    /// 9249bb7), kept only as an oracle for the no-regression corpus.
    fn substring_rule(line: &str) -> bool {
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
                part.split_once("://").is_some_and(|(_, rest)| {
                    rest.split('/').next().unwrap_or_default().contains('@')
                })
            })
            || lower
                .split(|c: char| c.is_whitespace() || "\"'=:,[]{}".contains(c))
                .any(|part| part.starts_with("sk-") && part.len() >= 16)
    }
    /// Realistic credential-bearing lines, written as programs and tools
    /// write them.
    const CREDENTIAL_LINES: &[&str] = &[
        // Environment dumps.
        "DB_PASSWORD=hunter2",
        "PGPASSWORD=hunter2",
        "MYSQL_ROOT_PASSWORD=hunter2",
        "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE",
        "GITHUB_TOKEN=ghp_16C7e42F292c6912E7710c838347Ae178B4a",
        "SECRET_KEY_BASE=8f3c0d6e1b2a",
        "JWT_SECRET=change-me",
        "export API_KEY=abc123",
        "declare -x REDIS_PASSWORD=\"hunter2\"",
        "NEXTAUTH_SECRET=Zm9vYmFy",
        "DJANGO_SECRET_KEY='django-insecure-abc'",
        "OAUTH_CLIENT_SECRET=abc",
        "SMTP_PASSWORD=hunter2",
        "TOKEN=abc",
        "2026-11-19T00:00:00.000000001Z env: POSTGRES_PASSWORD=hunter2",
        // Connection strings.
        "postgres://app:hunter2@db:5432/app",
        "postgresql://app:hunter2@db/app?sslmode=disable",
        "mysql://root:hunter2@127.0.0.1:3306/db",
        "mongodb+srv://u:p@cluster0.example.net/db",
        "redis://:hunter2@redis:6379/0",
        "amqp://guest:guest@rabbit:5672/",
        "Server=db;Database=app;User Id=sa;Password=hunter2;",
        "Host=db;Username=app;Password=hunter2",
        "jdbc:postgresql://db:5432/app?user=app&password=hunter2",
        "DATABASE_URL=postgres://app:hunter2@db/app",
        "connecting to mongodb://admin:hunter2@mongo:27017",
        // HTTP header dumps.
        "Authorization: Basic dXNlcjpodW50ZXIy",
        "authorization: Bearer abc.def.ghi",
        "> Authorization: token ghp_16C7e42F292c6912E7710c838347Ae178B4a",
        "Cookie: session=abc123; theme=dark",
        "Set-Cookie: sid=abc123; HttpOnly",
        "< set-cookie: __Host-limeos=abc; Path=/",
        "X-Api-Key: abc123",
        "Proxy-Authorization: Basic dXNlcjpodW50ZXIy",
        "X-Auth-Token: abc",
        "x-amz-security-token: FQoGZXIvYXdzEJr",
        r#""headers":{"authorization":"Bearer abc"}"#,
        "headers={'Authorization': 'Bearer abc'}",
        "Authorization:\t\t\tBearer\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\tabcDEF123456",
        // Docker inspect environment arrays.
        r#""Env": ["PATH=/usr/local/sbin", "POSTGRES_PASSWORD=hunter2"]"#,
        r#""Env":["MYSQL_ROOT_PASSWORD=hunter2","MYSQL_DATABASE=app"]"#,
        r#"            "GF_SECURITY_ADMIN_PASSWORD=hunter2","#,
        r#""Env": [ "TOKEN=abc" ],"#,
        r#"{"Env":[\"DB_PASSWORD=hunter2\"]}"#,
        // Kubernetes and ECS environment entries.
        r#"{"name":"DB_PASSWORD","value":"x"}"#,
        r#"{"name": "POSTGRES_PASSWORD", "value": "hunter2"}"#,
        r#"env: [{"name":"API_TOKEN","value":"abc"},{"name":"PORT","value":"8080"}]"#,
        r#"{"Name":"GITHUB_TOKEN","Value":"ghp_x"}"#,
        r#"{\"name\":\"DB_PASSWORD\",\"value\":\"x\"}"#,
        // YAML.
        "password: hunter2",
        "  db_password: \"hunter2\"",
        "api_key: 'abc'",
        "secretKey: abc",
        "- PASSWORD=hunter2",
        "    MYSQL_PASSWORD: hunter2",
        "client_secret: abc",
        "token: abc",
        // Command lines.
        "mysql -u root --password=hunter2 app",
        "mysqldump --user=root --password hunter2 app",
        "exec: /usr/bin/app --token s3cr3t --verbose",
        "/usr/bin/app --token=s3cr3t",
        "app --api-key abc",
        "app --secret=abc",
        "kubectl create secret generic db --from-literal=password=hunter2",
        "curl -H \"Authorization: Bearer abc\" https://example.com",
        "psql \"postgresql://app:hunter2@db/app\"",
        // SQL.
        "ALTER ROLE app PASSWORD 'x'",
        "ALTER ROLE app WITH LOGIN PASSWORD 'hunter2';",
        "CREATE USER app WITH PASSWORD 'hunter2'",
        "statement: ALTER USER postgres PASSWORD 'md5abc'",
        "CREATE USER app WITH PASSWORD \"hunter2\"",
        // Escaped JSON inside JSON.
        r#""body":"{\"user\":\"bob\",\"password\":\"hunter2\"}""#,
        r#"{"msg":"request","payload":"{\"api_key\":\"abc123\"}"}"#,
        r#"{"log":"{\\\"token\\\":\\\"s3cr3t\\\"}"}"#,
        r#"{"log":"{\"client_secret\": \"abc\"}\n"}"#,
        // Form and query parameters.
        "user[password]=x",
        "POST /users user[name]=bob&user[password]=hunter2",
        r#"params: {"user"=>{"email"=>"a@b", "password"=>"[FILTERED]"}}"#,
        "GET /cb?code=1&access_token=abc HTTP/1.1",
        "['password'] = 'x'",
        // Quoted values that start with whitespace or punctuation.
        r#"password=";a&b""#,
        r#"{"password":" hello"}"#,
        r#"{"password":"&x"}"#,
        r#"{"token":",abc"}"#,
        "secret=' leading space'",
        // Shapes after any whitespace.
        "Bearer                                            abcDEF123456",
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
        "sk-proj-abcdefghijklmnopqrstuvwxyz012345",
        "using key sk-ant-api03-abcdefghijklmnop0123456789",
    ];
    /// Lines with no credential, as the ordinary-lines test and container
    /// logs have them; some name a sensitive word in prose.
    const PROSE_LINES: &[&str] = &[
        "processed 4096 tokens",
        "session started",
        "secretary",
        "the token bucket refilled",
        "password reset email sent to user",
        "tokenizer loaded",
        "max_tokens=4096 prompt_tokens: 12",
        r#"{"usage":{"prompt_tokens":12,"completion_tokens":40}}"#,
        r#"{"event":"token_refreshed","user":"bob"}"#,
        r#"{"type":"session","id":5}"#,
        "msg=\"login failed for bob: password mismatch\"",
        "authorization failed for user bob",
        "the bearer of bad news",
        "Bearer token required",
        "secrets are mounted read-only",
        "active_sessions=5",
        "connected to postgres://db:5432/app",
        "risk-free reconciliation done in 12ms",
        "skipped 3 jobs",
        "GET /health 200",
        "Starting server on port 8080",
        "token refresh failed: upstream returned 401",
        "Secret rotation scheduled",
        "invalid password for user bob",
        "the Authorization header is missing",
    ];
    /// Lines the substring rule redacted that are intentionally emitted now.
    /// Reviewed: each names a sensitive word without giving it a value.
    const NO_LONGER_REDACTED: &[&str] = &[
        "processed 4096 tokens",
        "secretary",
        "the token bucket refilled",
        "password reset email sent to user",
        "tokenizer loaded",
        "max_tokens=4096 prompt_tokens: 12",
        r#"{"usage":{"prompt_tokens":12,"completion_tokens":40}}"#,
        r#"{"event":"token_refreshed","user":"bob"}"#,
        "msg=\"login failed for bob: password mismatch\"",
        "authorization failed for user bob",
        "the bearer of bad news",
        "Bearer token required",
        "secrets are mounted read-only",
        "token refresh failed: upstream returned 401",
        "Secret rotation scheduled",
        "invalid password for user bob",
        "the Authorization header is missing",
    ];
    /// The corpus: every credential line, each in a few surroundings, and the
    /// prose lines.
    fn corpus() -> Vec<String> {
        let mut lines = Vec::new();
        for line in CREDENTIAL_LINES.iter().chain(REPRODUCTIONS) {
            lines.push((*line).to_owned());
            lines.push(format!("2026-11-19T00:00:00.000000001Z {line}"));
            lines.push(format!("app[1]: {line}"));
            lines.push(format!("{line} (retrying)"));
        }
        // The reported forms with other key names.
        for key in ["password", "passwd", "token", "secret", "api_key", "apikey"] {
            let upper = key.to_ascii_uppercase();
            lines.extend([
                format!(r#""body":"{{\"user\":\"bob\",\"{key}\":\"hunter2\"}}""#),
                format!(r#"{{"m":"{{\"{key}\" : \"hunter2\"}}"}}"#),
                format!("SET {upper} 'x'"),
                format!("--{key} x"),
                format!("--{key}\tx"),
                format!(r#"{{"name":"DB_{upper}","value":"x"}}"#),
                format!(r#"{{"name":"{upper}", "value": "x"}}"#),
                format!("user[{key}]=x"),
                format!(r#"{key}=";a&b""#),
                format!(r#"{{"{key}":" hello"}}"#),
                format!("{upper}={key}-value"),
                format!("{key}: \"{key}\""),
            ]);
        }
        lines.extend(PROSE_LINES.iter().map(|line| (*line).to_owned()));
        lines
    }
    fn redacted(line: &str) -> bool {
        filtered(line) == CREDENTIAL_REDACTED
    }
    #[test]
    fn every_corpus_line_the_substring_rule_redacted_is_still_redacted() {
        let corpus = corpus();
        assert!(corpus.len() >= 450, "{}", corpus.len());
        let emitted: Vec<_> = corpus
            .iter()
            .filter(|line| substring_rule(line) && !redacted(line))
            .map(String::as_str)
            .collect();
        assert_eq!(emitted, NO_LONGER_REDACTED);
        for line in CREDENTIAL_LINES {
            assert!(redacted(line), "{line:?}");
        }
        for line in PROSE_LINES {
            assert!(!redacted(line), "{line:?}");
        }
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
