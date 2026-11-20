//! Bounded container log reads: the newest whole lines, terminal controls
//! stripped, private keys and credential-bearing lines redacted.
//!
//! Redaction is line based, and a line that names a credential is redacted
//! unless it is plainly prose: a redactor that lists the forms a secret can
//! take after its name (assignments, records, options, keywords) keeps
//! missing forms, and a missed secret is worse than a hidden line.
//!
//! A *key* is a maximal run of ASCII letters, digits, `_`, `-` and `.`. A
//! key is a *sensitive word* when it contains, in any case, one of the
//! substring rule's markers this rule replaced — `password`, `passwd`,
//! `passphrase`, `secret`, `api_key`, `api-key`, `apikey`, `token`,
//! `authorization`, `bearer`, `ghp_`, `github_pat_`, `sk-ant-`, `sk-proj-`,
//! `x-amz-credential`, `x-amz-signature`, `x-goog-signature` — or one of
//! `credential`, `cookie`, `signature`, `session`, `auth` and `identified`
//! (SQL's `IDENTIFIED BY 'x'`), or holds `key` after `api` or `private`
//! with only digits, `_`, `-` or `.` between (`privatekey`, `API__KEY`).
//! Detection is a plain substring test, so it finds every word the
//! substring rule found and more: `db_password`, `SECRETKEY`,
//! `mysecretvalue`, `tokenfile`, `Set-Cookie`, but also `secretary`,
//! `tokenizer` and `author`, which the prose test below then shows. A line
//! is redacted when:
//! - (a) **it names a sensitive word and is not plain prose there**. Plain
//!   prose means both of:
//!   - each sensitive word is one plain word: at most 15 ASCII letters,
//!     upper case only first, before any sentence-ending dots (`Password`,
//!     `tokens`, `token.`, `secretary`). In `--password`, `DB_PASSWORD`,
//!     `PASSWORD`, `SECRETKEY`, `accessToken`, `token2` or a compound of 16
//!     letters or more the term is part of a name, never prose;
//!   - for every sensitive word, the text from its last letter to the next
//!     sensitive word or the line's end holds only spaces, words of at most
//!     15 lower-case ASCII letters, and `.`, `,`, `;`, `!` or `?` each
//!     followed by a space or the line's end. Anything else there (a digit,
//!     an upper-case letter, a quote, `=`, `:`, `-`, `_`, `/`, `\`, `@`,
//!     `%`, `+`, a bracket or brace, a tab, a non-ASCII byte, a longer word)
//!     could be or introduce a value.
//!
//!   What precedes the first sensitive word does not matter;
//! - (b) **it holds a value of a known secret shape** anywhere, after any
//!   amount of whitespace: a bearer credential, a JWT, cloud and service key
//!   prefixes, URL userinfo.
//!
//! Kept: `processed 4096 tokens`, `session started`, `password reset email
//! sent to user`, `invalid password for user bob`, `the token bucket
//! refilled`, `tokenizer loaded`. Redacted though harmless, by design:
//! `--token-ttl 3600`, `Secret "db-creds" not found`,
//! `{"metric":"session","value":42}`, `max_tokens=4096`, `token refresh
//! failed: upstream returned 401`, `secrets are mounted read-only` (the
//! hyphen), `SECRET_KEY not set`, `Author: bob`, a sensitive word before
//! non-ASCII text, `limeos_core::session: started`.
//!
//! Bounds: one forward pass finds the keys; each key is tested for every
//! term at each of its positions (a fixed number of bytes per position; the
//! digits and separators between `api` or `private` and `key` follow one
//! letter each and are read once) and checked to be a plain word; each
//! sensitive word closes the span after the previous one, and the spans do
//! not overlap, so each is read once; the shape checks read a bounded number
//! of bytes from each position. The cost is linear in the line and no input
//! can make it panic.
//!
//! Known limits. A line is kept when every sensitive word on it is a plain
//! word followed by lower-case prose, so it shows a secret made of
//! lower-case words given bare after its name with no quoting, operator or
//! option dashes (`password hunter`), a secret that is exactly a
//! capitalised or lower-case sensitive word (`password Secret`: the next
//! sensitive word ends the span), and a secret that comes *before* the
//! sensitive word with only prose after it (`hunter2 is the password`).
//! Also missed: a secret on the line after its key (YAML block style
//! `password: |`, or `- name: DB_PASSWORD` then `value: x`); a key written
//! with look-alike Unicode letters (non-ASCII *after* a sensitive word does
//! redact); a value wrapped in base64 or another encoding (a basic auth
//! header is caught by its key, not its value); a secret passed with no
//! sensitive word on its line (`mysql -phunter2`).
//!
//! Key blocks with their BEGIN/END markers are reliable; a key body without markers (its BEGIN dropped by the Engine's
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
/// Whether a line carries a credential: a value with a known secret shape,
/// or a sensitive word followed by anything but plain prose (the module
/// documentation states both). Each scan reads the line's bytes once or a
/// bounded number of bytes per position, so the cost is linear in the line.
fn credential(line: &str) -> bool {
    let bytes = line.trim_end_matches(['\n', '\r']).as_bytes();
    named_without_prose(bytes)
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
/// The count of spaces and tabs at the start of `rest`.
fn spaces(rest: &[u8]) -> usize {
    rest.iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t'))
        .count()
}
/// The longest word of lower-case letters plain prose may contain.
const MAX_WORD: usize = 15;
/// Whether `bytes` names a sensitive word and is not plain prose there:
/// a sensitive word is not a plain word, or the text after one, up to the
/// next or the line's end, is not prose. One forward pass over the line's
/// keys (maximal runs of letters, digits, `_`, `-` and `.`); each sensitive
/// word closes the span after the previous one, which `prose` reads once, so
/// every byte is read a bounded number of times.
fn named_without_prose(bytes: &[u8]) -> bool {
    // Where the span after the last sensitive word so far starts.
    let mut after = None;
    let mut at = 0;
    while at < bytes.len() {
        let len = bytes
            .get(at..)
            .unwrap_or_default()
            .iter()
            .take_while(|&&b| key_byte(b))
            .count();
        if len == 0 {
            at += 1;
            continue;
        }
        let key = bytes.get(at..at + len).unwrap_or_default();
        if sensitive_word(key) {
            let Some(letters) = plain_word(key) else {
                return true;
            };
            if after.is_some_and(|from| !prose(bytes, from, at)) {
                return true;
            }
            after = Some(at + letters);
        }
        at += len;
    }
    after.is_some_and(|from| !prose(bytes, from, bytes.len()))
}
/// If `key` is one plain word — at most `MAX_WORD` ASCII letters, upper
/// case only first (`Password`, `tokens`), before any sentence-ending dots —
/// the count of its letters. The dots are left to the prose test.
fn plain_word(key: &[u8]) -> Option<usize> {
    let end = key
        .iter()
        .rposition(|&b| b != b'.')
        .map_or(0, |last| last + 1);
    let mut letters = key.get(..end).unwrap_or_default().iter();
    (end <= MAX_WORD
        && letters.next().is_some_and(u8::is_ascii_alphabetic)
        && letters.all(u8::is_ascii_lowercase))
    .then_some(end)
}
/// Whether `line[from..to]` is plain prose: spaces, words of at most
/// `MAX_WORD` lower-case ASCII letters, and `.`, `,`, `;`, `!` or `?` each
/// followed by a space or the line's end.
fn prose(line: &[u8], from: usize, to: usize) -> bool {
    let mut word = 0;
    for at in from..to {
        match line.get(at).copied() {
            Some(b'a'..=b'z') if word < MAX_WORD => word += 1,
            Some(b' ') => word = 0,
            Some(b'.' | b',' | b';' | b'!' | b'?')
                if matches!(line.get(at + 1), None | Some(b' ')) =>
            {
                word = 0
            }
            _ => return false,
        }
    }
    true
}
/// Terms that make a key a sensitive word when it contains one, in any case.
/// First the markers of the substring rule this module replaced (as of
/// 9249bb7, `bearer ` without its space), which a test keeps in step with
/// that rule, then the words this rule adds. `identified` is SQL's
/// `IDENTIFIED BY 'x'`, a password with no other word naming it.
const SENSITIVE_TERMS: &[&[u8]] = &[
    b"password",
    b"passwd",
    b"passphrase",
    b"secret",
    b"api_key",
    b"api-key",
    b"apikey",
    b"token",
    b"authorization",
    b"bearer",
    b"ghp_",
    b"github_pat_",
    b"sk-ant-",
    b"sk-proj-",
    b"x-amz-credential",
    b"x-amz-signature",
    b"x-goog-signature",
    b"credential",
    b"cookie",
    b"signature",
    b"session",
    b"auth",
    b"identified",
];
/// Whether `key` is a sensitive word: it contains a term of
/// `SENSITIVE_TERMS`, or `key` after `api` or `private` with only digits,
/// `_`, `-` or `.` between (`privatekey`, `private.key`, `API__KEY`), in any
/// case. Each position is compared with a fixed number of bytes, and the run
/// between `api` or `private` and `key` follows a letter, so no run is read
/// twice.
fn sensitive_word(key: &[u8]) -> bool {
    let starts = |at: usize, word: &[u8]| {
        key.get(at..at + word.len())
            .is_some_and(|here| here.eq_ignore_ascii_case(word))
    };
    (0..key.len()).any(|at| {
        SENSITIVE_TERMS.iter().any(|term| starts(at, term))
            || [&b"api"[..], b"private"].iter().any(|word| {
                let after = at + word.len();
                starts(at, word) && {
                    let gap = key
                        .get(after..)
                        .unwrap_or_default()
                        .iter()
                        .take_while(|&&b| b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.'))
                        .count();
                    starts(after + gap, b"key")
                }
            })
    })
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
            "{",
            "}",
            "%3D",
            "%3a",
            "name",
            "Value",
            "PASSWORD",
            "IDENTIFIED",
            "BY",
            "WITH",
            "\\\\\\\"",
            "SECRETKEY",
            "private",
            "secretary",
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
            "{",
            "}",
            "\\",
            "\\\\\\\"token\\\\\\\":",
            "{\"name\":\"PASSWORD\",",
            "name: PASSWORD, ",
            "%3D",
            "token%3D",
            "PASSWORD ",
            "IDENTIFIED BY ",
            "IDENTIFIED WITH x ",
            "--db-token ",
            "token a ",
            "password. ",
            "csrftoken",
            "tokens",
            "Session, ",
            "api_",
            "private.",
            "api__key",
            "SECRETKEY=",
            "secretary ",
        ] {
            let line = unit.repeat(CONTAINER_LOG_BYTES / unit.len() - 1);
            decode_all(line.as_bytes(), true).unwrap();
        }
    }
    /// Lines the reviewers found emitted by an earlier rule while the
    /// substring rule redacted them, over four review rounds.
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
        // Second and third rounds.
        "DB_PASSWORD=;a&b",
        r#"{"value":"hunter2","name":"DB_PASSWORD"}"#,
        r#"{"name":"DB_PASSWORD","type":"string","value":"hunter2"}"#,
        "name: DB_PASSWORD, value: x",
        "{ name: 'DB_PASSWORD', value: 'x' }",
        "PASSWORD hunter2",
        "password%3Dx",
        r#"{"log":"{\\\\\\\"token\\\\\\\":\\\\\\\"s3cr3t\\\\\\\"}"}"#,
        "CREATE USER 'app'@'%' IDENTIFIED BY 'hunter2'",
        "ALTER USER app IDENTIFIED WITH caching_sha2_password BY 'hunter2'",
        // Fourth round.
        "--s3-secret-access-key hunter2",
        r#"{"value":"}hunter2","name":"DB_PASSWORD"}"#,
        r#"{"name":"DB_PASSWORD","value":"}hunter2"}"#,
        "alter role app password 'x'",
    ];
    #[test]
    fn reported_misses_are_redacted() {
        for line in REPRODUCTIONS {
            assert_eq!(filtered(line), CREDENTIAL_REDACTED, "{line:?}");
        }
    }
    /// Lines and whether the prose rule redacts them, as the owner listed
    /// them and as earlier reviews found them.
    const OUTCOMES: &[(&str, bool)] = &[
        // Kept: only lower-case prose follows each sensitive word.
        ("processed 4096 tokens", false),
        ("session started", false),
        ("password reset email sent to user", false),
        ("the token bucket refilled", false),
        ("secretary", false),
        ("tokenizer loaded", false),
        ("authorization failed for user bob", false),
        ("invalid password for user bob", false),
        ("Bearer token required", false),
        ("the Authorization header is missing", false),
        ("Secret rotation scheduled", false),
        ("password reset. check your inbox, then sign in!", false),
        ("the user was identified by email", false),
        ("session started for the token.", false),
        ("Password expired; Tokens revoked.", false),
        ("the bearer of bad news", false),
        ("the author replied", false),
        // Kept: no sensitive word.
        ("id=550e8400-e29b-41d4-a716-446655440000 ready", false),
        ("connected to postgres://db:5432/app", false),
        ("risk-free reconciliation done in 12ms", false),
        ("skipped 3 jobs", false),
        ("2026-11-19T00:00:00.000000001Z GET /health 200", false),
        // Redacted: a value could follow the sensitive word.
        ("--s3-secret-access-key hunter2", true),
        (r#"{"value":"}hunter2","name":"DB_PASSWORD"}"#, true),
        (r#"{"name":"DB_PASSWORD","value":"}hunter2"}"#, true),
        ("alter role app password 'x'", true),
        ("ALTER ROLE app PASSWORD 'x'", true),
        ("PASSWORD hunter2", true),
        ("DB_PASSWORD=;a&b", true),
        ("DB_PASSWORD2=hunter2", true),
        ("password%3Dx", true),
        ("name: DB_PASSWORD, value: x", true),
        ("{ name: 'DB_PASSWORD', value: 'x' }", true),
        ("password hunter2secret", true),
        ("password reset.hunter", true),
        ("password\thunter", true),
        ("password abcdefghijklmnop", true),
        // Redacted: the sensitive word is in a key that is not a plain word.
        ("--password x", true),
        ("app --api-key abc", true),
        ("PASSWORD hunter", true),
        ("value=hunter2, name=PASSWORD", true),
        ("password SECRET", true),
        ("accessToken expired", true),
        // Redacted: a term inside a compound name, which only a substring
        // test finds, or inside a word of 16 letters or more.
        ("SECRETKEY=HuNter2!", true),
        ("PASSWORDFILE=/run/x", true),
        ("mysecretvalue=abc", true),
        ("tokenfile: abc123", true),
        ("sessionkey=abc", true),
        ("Secretkey=hunter", true),
        ("API__KEY=abc", true),
        ("passwordmanagement enabled", true),
        ("the obsessionwithtokens grew", true),
        // Redacted though harmless: the accepted cost.
        ("secrets are mounted read-only", true),
        ("--token-ttl 3600", true),
        ("--session-timeout 30", true),
        (r#"Secret "db-creds" not found"#, true),
        (r#"{"metric":"session","value":42}"#, true),
        (r#"{"name":"session duration","value":42}"#, true),
        ("max_tokens=4096 prompt_tokens: 12", true),
        ("msg=\"login failed for bob: password mismatch\"", true),
        ("token refresh failed: upstream returned 401", true),
        (r#"{"event":"token_refreshed","user":"bob"}"#, true),
        (
            r#"{"usage":{"prompt_tokens":12,"completion_tokens":40}}"#,
            true,
        ),
        (r#"{"type":"session","id":5}"#, true),
        ("active_sessions=5", true),
        ("limeos_core::session: started", true),
        ("Ünïcödé session 😀 started", true),
        ("SECRET_KEY not set", true),
        ("Author: bob", true),
    ];
    #[test]
    fn listed_lines_have_their_listed_outcome() {
        let digest = "a3ed95caeb02ffe68cdd9fd84406680ae93d633cb16422d00e8a7c22955b46d4";
        let shown = [
            format!("pulling docker.io/library/nginx@sha256:{digest}"),
            format!("image=nginx@sha256:{digest}"),
        ];
        let lines = OUTCOMES
            .iter()
            .map(|&(line, redact)| (line.to_owned(), redact))
            .chain(shown.into_iter().map(|line| (line, false)));
        for (line, redact) in lines {
            let expected = if redact {
                CREDENTIAL_REDACTED.to_owned()
            } else {
                format!("{line}\n")
            };
            assert_eq!(filtered(&line), expected, "{line:?}");
        }
    }
    /// The documented limits, pinned so that a change to them is deliberate:
    /// each carries a secret and is shown.
    const KNOWN_LIMITS: &[&str] = &[
        // Lower-case words given bare after the sensitive word.
        "password hunter",
        // A secret that is itself a sensitive word ends the span.
        "password Secret",
        // A secret before the sensitive word, only prose after it.
        "hunter2 is the password",
        // No sensitive word on the line.
        "mysql -phunter2",
        // The value on the next line: this line has only the key.
        "password",
    ];
    #[test]
    fn documented_limits_are_shown() {
        for line in KNOWN_LIMITS {
            assert_eq!(filtered(line), format!("{line}\n"), "{line:?}");
        }
    }
    /// A small xorshift generator, so the generated tests are repeatable.
    fn generator(mut state: u64) -> impl FnMut(usize) -> usize {
        move |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        }
    }
    /// Sensitive words as plain words, as prose writes them.
    const SENSITIVE_WORDS: &[&str] = &[
        "password",
        "passwd",
        "passphrase",
        "secret",
        "token",
        "apikey",
        "privatekey",
        "credential",
        "credentials",
        "cookie",
        "signature",
        "session",
        "sessionid",
        "authorization",
        "auth",
        "identified",
        "tokens",
        "secrets",
        "passwords",
        "Password",
        "Tokens",
        "csrftoken",
    ];
    /// Sensitive words inside keys that are not plain words: never prose.
    const SENSITIVE_KEYS: &[&str] = &[
        "TOKEN",
        "DB_PASSWORD",
        "accessToken",
        "X-Api-Key",
        "api_key",
        "PGPASSWORD",
        "Set-Cookie",
        "private_key",
        "--password",
        "-token",
        "token2",
        "auth0",
        "password.file",
    ];
    /// Lower-case words with no sensitive word in them.
    const PROSE_WORDS: &[&str] = &[
        "alpha", "bravo", "charlie", "delta", "echo", "is", "a", "the", "for", "user", "bob",
        "mismatch", "reset", "rotated", "expired", "missing", "q",
    ];
    /// What may stand before the first sensitive word: anything.
    const PREFIXES: &[&str] = &[
        "",
        "INFO ",
        "2026-11-19T00:00:00.000000001Z ",
        "app[1]: ",
        "{\"msg\":\"",
        "Error 42: ",
        "The Thing ",
        "x=1 ",
    ];
    /// Pieces that are not plain prose: anything but lower-case letters and
    /// spaces, punctuation not followed by a space, and an over-long word.
    const NOT_PROSE: &[&str] = &[
        "0",
        "7",
        "Q",
        "Z",
        "\"",
        "'",
        "`",
        "=",
        ":",
        "-",
        "_",
        "/",
        "\\",
        "@",
        "%",
        "+",
        "[",
        "]",
        "{",
        "}",
        "(",
        ")",
        "<",
        ">",
        "#",
        "$",
        "&",
        "*",
        "|",
        "~",
        "^",
        "\t",
        "é",
        "😀",
        "\u{a0}",
        ".x",
        ",x",
        "!x",
        "?x",
        ";x",
        "abcdefghijklmnop",
    ];
    #[test]
    fn a_sensitive_word_followed_by_anything_but_lower_case_prose_is_redacted() {
        let mut next = generator(0x2545_f491_4f6c_dd1d);
        let pick =
            |next: &mut dyn FnMut(usize) -> usize, from: &[&'static str]| from[next(from.len())];
        for _ in 0..4000 {
            let prefix = pick(&mut next, PREFIXES);
            let word = if next(2) == 0 {
                pick(&mut next, SENSITIVE_WORDS)
            } else {
                pick(&mut next, SENSITIVE_KEYS)
            };
            let mut value: Vec<String> = (0..next(4))
                .map(|_| pick(&mut next, PROSE_WORDS).to_owned())
                .collect();
            let other = pick(&mut next, NOT_PROSE);
            let at = next(value.len() + 1);
            // Glued to a neighbouring word, or a word of its own.
            match value.get_mut(at) {
                Some(word) if next(2) == 0 => word.insert_str(0, other),
                _ => value.insert(at, other.to_owned()),
            }
            // Directly after the sensitive word only what cannot extend it.
            let glue = at == 0 && !other.starts_with(|c: char| c.is_ascii_alphabetic());
            let gap = if glue && next(2) == 0 { "" } else { " " };
            let line = format!("{prefix}{word}{gap}{}", value.join(" "));
            assert_eq!(filtered(&line), CREDENTIAL_REDACTED, "{line:?}");
        }
    }
    /// A sensitive word or key after any prefix, then only lower-case prose.
    fn prose_after(next: &mut dyn FnMut(usize) -> usize, words: &[&str]) -> String {
        let mut line = format!(
            "{}{}",
            PREFIXES[next(PREFIXES.len())],
            words[next(words.len())]
        );
        for _ in 0..next(6) {
            line.push_str([" ", " ", ", ", ". ", "; "][next(5)]);
            line.push_str(PROSE_WORDS[next(PROSE_WORDS.len())]);
        }
        line.push_str(["", "", ".", "!", "?", " "][next(6)]);
        line
    }
    #[test]
    fn a_sensitive_word_followed_only_by_lower_case_prose_is_shown() {
        let mut next = generator(0x9e37_79b9_7f4a_7c15);
        for _ in 0..4000 {
            let line = prose_after(&mut next, SENSITIVE_WORDS);
            assert_eq!(filtered(&line), format!("{line}\n"), "{line:?}");
        }
    }
    #[test]
    fn a_sensitive_word_in_a_key_that_is_not_a_plain_word_is_never_prose() {
        let mut next = generator(0x6a09_e667_f3bc_c908);
        for _ in 0..2000 {
            let line = prose_after(&mut next, SENSITIVE_KEYS);
            assert_eq!(filtered(&line), CREDENTIAL_REDACTED, "{line:?}");
        }
    }
    /// The decision of the substring rule this module replaced (as of
    /// 9249bb7), kept only as an oracle for the no-regression corpus.
    fn substring_rule(line: &str) -> bool {
        let lower = line.to_ascii_lowercase();
        ORACLE_MARKERS.iter().any(|marker| lower.contains(marker))
            || lower.split_whitespace().any(|part| {
                part.split_once("://").is_some_and(|(_, rest)| {
                    rest.split('/').next().unwrap_or_default().contains('@')
                })
            })
            || lower
                .split(|c: char| c.is_whitespace() || "\"'=:,[]{}".contains(c))
                .any(|part| part.starts_with("sk-") && part.len() >= 16)
    }
    const ORACLE_MARKERS: &[&str] = &[
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
    ];
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
    /// Lines the substring rule redacted that the prose rule shows. Reviewed
    /// line by line: each is plain prose after its sensitive word, or has
    /// none, and carries no secret.
    const NO_LONGER_REDACTED: &[&str] = &[
        "processed 4096 tokens",
        "password reset email sent to user",
        "the token bucket refilled",
        "secretary",
        "tokenizer loaded",
        "authorization failed for user bob",
        "invalid password for user bob",
        "Bearer token required",
        "the Authorization header is missing",
        "Secret rotation scheduled",
        "password reset. check your inbox, then sign in!",
        "session started for the token.",
        "Password expired; Tokens revoked.",
        "the bearer of bad news",
    ];
    /// The corpus: every credential line and reported miss, each in a few
    /// surroundings, the listed lines and the systematic variants.
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
        lines.extend(OUTCOMES.iter().map(|(line, _)| (*line).to_owned()));
        lines.extend(variants());
        lines
    }
    /// Keys for the systematic variants, as programs write them.
    const VARIANT_KEYS: &[&str] = &[
        "password",
        "db_password",
        "PGPASSWORD",
        "passwd",
        "passphrase",
        "secret",
        "client_secret",
        "token",
        "access_token",
        "accessToken",
        "api_key",
        "X-Api-Key",
        "apikey",
        "authorization",
        "cookie",
        "session",
        "private_key",
        "credentials",
        "signature",
    ];
    /// A double quote escaped for JSON nested `depth` levels inside JSON
    /// strings: 0, 1, 3, 7, ... backslashes before it.
    fn quote_at(depth: u32) -> String {
        format!("{}\"", "\\".repeat((1 << depth) - 1))
    }
    /// Every sensitive key in every form earlier rules enumerated
    /// (assignment, name/value record, option, bare keyword and SQL in both
    /// cases), quoted and not, escaped zero to six levels deep, with the
    /// record's fields in each order and with braces inside its strings.
    /// Each line carries a credential.
    fn variants() -> Vec<String> {
        let names = ["name", "key", "env", "variable", "Name", "Key", "NAME"];
        let values = ["value", "Value", "val"];
        let mut lines = Vec::new();
        for (i, key) in VARIANT_KEYS.iter().enumerate() {
            let upper = key.to_ascii_uppercase().replace('-', "_");
            let lower = key.to_ascii_lowercase();
            let (name, value) = (names[i % names.len()], values[i % values.len()]);
            // Assignments, through every operator.
            for op in ["=", ":", " = ", ": ", ":=", " => ", "%3D", "%3A", "%3d"] {
                for v in ["hunter2", ";a&b", "&x", ",abc"] {
                    lines.push(format!("{key}{op}{v}"));
                }
                lines.push(format!("'{key}'{op}'hunter2'"));
                for depth in 0..7 {
                    let q = quote_at(depth);
                    lines.push(format!("{q}{key}{q}{op}{q}hunter2{q}"));
                    lines.push(format!("{{{q}user{q}:{q}bob{q},{q}{key}{q}{op}{q} x{q}}}"));
                }
            }
            // Name/value records, in each field order, with and without a
            // brace inside the value.
            let orders: [&[usize]; 4] = [&[0, 1], &[1, 0], &[0, 2, 1], &[1, 2, 0]];
            for order in orders {
                for secret in ["hunter2", "}hunter2", "{hunter2", "x}"] {
                    for depth in 0..7 {
                        let q = quote_at(depth);
                        let fields = [
                            format!("{q}{name}{q}:{q}{upper}{q}"),
                            format!("{q}{value}{q}:{q}{secret}{q}"),
                            format!("{q}type{q}:{q}string{q}"),
                        ];
                        let record: Vec<_> = order.iter().map(|&f| fields[f].as_str()).collect();
                        lines.push(format!("{{{}}}", record.join(",")));
                    }
                }
                let node = [
                    format!("{name}: '{upper}'"),
                    format!("{value}: 'hunter2'"),
                    "type: 'string'".to_owned(),
                ];
                let record: Vec<_> = order.iter().map(|&f| node[f].as_str()).collect();
                lines.push(format!("{{ {} }}", record.join(", ")));
                for op in [": ", "="] {
                    let bare = [
                        format!("{name}{op}{upper}"),
                        format!("{value}{op}hunter2"),
                        format!("type{op}string"),
                    ];
                    let record: Vec<_> = order.iter().map(|&f| bare[f].as_str()).collect();
                    lines.push(record.join(", "));
                }
            }
            // Options, with the sensitive key anywhere in the name.
            for form in [
                "--{} x",
                "-{} x",
                "--db-{} x",
                "--{}\tx",
                "--{} 'x'",
                "--{} \"x\"",
                "--{}-access-key hunter2",
                "--s3-{}-access-key hunter2",
                "--{}-file /run/secrets/x",
                "-{}=x",
            ] {
                lines.push(form.replace("{}", key));
            }
            // Bare keywords and SQL, in upper and lower case.
            for case in [&upper, &lower] {
                for form in [
                    "{} hunter2",
                    "{} 'hunter2'",
                    "{} \"hunter2\"",
                    "WITH {} hunter2",
                    "ALTER ROLE app {} 'x'",
                    "alter role app {} 'x'",
                    "alter role app with login {} 'hunter2';",
                    "create user app with {} \"hunter2\"",
                    "statement: alter user postgres {} 'md5abc'",
                ] {
                    lines.push(form.replace("{}", case));
                }
            }
        }
        lines.extend(compounds());
        lines
    }
    /// `word` in lower case, upper case, with a capital first letter, and in
    /// alternating case.
    fn cased(word: &str) -> [String; 4] {
        let mut capital = word.to_owned();
        if let Some(first) = capital.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        let alternating = word
            .chars()
            .enumerate()
            .map(|(i, c)| {
                if i % 2 == 0 {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            })
            .collect();
        [
            word.to_owned(),
            word.to_ascii_uppercase(),
            capital,
            alternating,
        ]
    }
    /// Every sensitive term inside a compound name, which only a substring
    /// test finds: at its start, middle and end, in each case, in
    /// assignment, option and record forms. Each line carries a credential.
    fn compounds() -> Vec<String> {
        let mut lines = Vec::new();
        for term in SENSITIVE_TERMS {
            let term = std::str::from_utf8(term).unwrap();
            for compound in [
                format!("{term}file"),
                format!("my{term}value"),
                format!("db{term}"),
            ] {
                for key in cased(&compound) {
                    for form in [
                        "{}=HuNter2!",
                        "{}=/run/x",
                        "{}: abc123",
                        "\"{}\":\"hunter2\"",
                        "export {}='x'",
                        "--{} hunter2",
                        "--{}=x",
                        "-{} 'x'",
                        r#"{"name":"{}","value":"x"}"#,
                        r#"{"value":"x","name":"{}"}"#,
                        "name: {}, value: x",
                    ] {
                        lines.push(form.replace("{}", &key));
                    }
                }
            }
        }
        lines
    }
    #[test]
    fn every_systematic_variant_is_redacted() {
        let lines = variants();
        assert!(lines.len() >= 6000, "{}", lines.len());
        let missed: Vec<_> = lines.iter().filter(|line| !redacted(line)).collect();
        assert!(missed.is_empty(), "{} missed: {missed:#?}", missed.len());
    }
    #[test]
    fn escaping_and_nesting_at_any_depth_redact() {
        // JSON escaped to any depth: the backslashes follow the key.
        for depth in 0..12 {
            let q = quote_at(depth);
            for line in [
                format!("{q}token{q}:{q}s3cr3t{q}"),
                format!("{{{q}name{q}:{q}DB_PASSWORD{q},{q}value{q}:{q}x{q}}}"),
                format!("{{{q}value{q}:{q}}}x{q},{q}name{q}:{q}DB_PASSWORD{q}}}"),
            ] {
                assert!(redacted(&line), "{depth}: {line}");
            }
        }
        // However deeply nested, and whatever objects lie between fields.
        let deep = "{".repeat(10_000);
        let shut = "}".repeat(10_000);
        for line in [
            format!(r#"{deep}"name":"DB_PASSWORD","value":"x""#),
            format!(r#""name":"DB_PASSWORD",{deep}{shut},"value":"x""#),
            format!(r#"{deep}"name":"DB_PASSWORD"{shut},"value":"x""#),
            r#"[{"name":"DB_PASSWORD","valueFrom":{"ref":"db"}},{"name":"PORT","value":"8080"}]"#
                .to_owned(),
        ] {
            assert!(redacted(&line), "{line}");
        }
        // An empty value is redacted too: the rule does not read values.
        for line in [
            r#"{"password":""}"#,
            "password=",
            "password:   ",
            r#"{"name":"DB_PASSWORD","value":""}"#,
            "--password-file",
        ] {
            assert!(redacted(line), "{line}");
        }
    }
    fn redacted(line: &str) -> bool {
        filtered(line) == CREDENTIAL_REDACTED
    }
    /// An independent, coarser reading of plain prose: after the first of the
    /// substring rule's markers, only lower-case letters, spaces and
    /// sentence punctuation.
    fn prose_after_marker(line: &str) -> bool {
        let lower = line.to_ascii_lowercase();
        let first = ORACLE_MARKERS
            .iter()
            .filter_map(|marker| lower.find(marker).map(|at| at + marker.len()))
            .min();
        first.is_some_and(|at| {
            lower[at..]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b" .,;!?".contains(&b))
        })
    }
    /// The plain-prose definition of the module documentation, written
    /// independently of the scan: `None` when the line names no sensitive
    /// word, otherwise whether every sensitive word is a plain word followed
    /// by prose up to the next one or the line's end.
    fn prose_definition(line: &str) -> Option<bool> {
        let bytes = line.as_bytes();
        let key_byte = |b: u8| b.is_ascii_alphanumeric() || b"_-.".contains(&b);
        let mut keys = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let start = at;
            while at < bytes.len() && key_byte(bytes[at]) {
                at += 1;
            }
            if at > start {
                keys.push((start, at));
            } else {
                at += 1;
            }
        }
        let sensitive: Vec<_> = keys
            .into_iter()
            .filter(|&(start, end)| {
                let key = line[start..end].to_ascii_lowercase();
                let term = SENSITIVE_TERMS
                    .iter()
                    .any(|term| key.contains(std::str::from_utf8(term).unwrap()));
                let api_key = ["api", "private"].iter().any(|word| {
                    key.match_indices(word).any(|(at, _)| {
                        key[at + word.len()..]
                            .trim_start_matches(|c: char| c.is_ascii_digit() || "_-.".contains(c))
                            .starts_with("key")
                    })
                });
                term || api_key
            })
            .collect();
        if sensitive.is_empty() {
            return None;
        }
        Some(sensitive.iter().enumerate().all(|(n, &(start, end))| {
            let word = line[start..end].trim_end_matches('.');
            let plain = (1..=15).contains(&word.len())
                && word.bytes().all(|b| b.is_ascii_alphabetic())
                && word.bytes().skip(1).all(|b| b.is_ascii_lowercase());
            let to = sensitive.get(n + 1).map_or(line.len(), |&(next, _)| next);
            let span = &line[start + word.len()..to];
            // Words of at most 15 lower-case letters, each with at most one
            // punctuation mark after it, which a space or the line's end
            // follows.
            let tokens: Vec<_> = span.split(' ').collect();
            plain
                && tokens.iter().enumerate().all(|(i, token)| {
                    let (letters, mark) = match token.strip_suffix(['.', ',', ';', '!', '?']) {
                        Some(letters) => (letters, true),
                        None => (*token, false),
                    };
                    letters.len() <= 15
                        && letters.bytes().all(|b| b.is_ascii_lowercase())
                        && !(mark && i + 1 == tokens.len() && to != line.len())
                })
        }))
    }
    #[test]
    fn every_corpus_line_the_substring_rule_redacted_is_still_redacted_unless_plain_prose() {
        let corpus = corpus();
        assert!(corpus.len() >= 8000, "{}", corpus.len());
        let emitted: Vec<_> = corpus
            .iter()
            .filter(|line| substring_rule(line) && !redacted(line))
            .map(String::as_str)
            .collect();
        for line in &emitted {
            assert_eq!(prose_definition(line), Some(true), "{line:?}");
            assert!(NO_LONGER_REDACTED.contains(line), "{line:?}");
        }
        assert_eq!(emitted, NO_LONGER_REDACTED);
        for line in NO_LONGER_REDACTED {
            assert!(prose_after_marker(line), "{line:?}");
        }
        for line in CREDENTIAL_LINES {
            assert!(redacted(line), "{line:?}");
        }
    }
    #[test]
    fn every_compound_name_is_redacted() {
        let lines = compounds();
        assert_eq!(lines.len(), SENSITIVE_TERMS.len() * 3 * 4 * 11);
        let missed: Vec<_> = lines.iter().filter(|line| !redacted(line)).collect();
        assert!(missed.is_empty(), "{} missed: {missed:#?}", missed.len());
    }
    #[test]
    fn detection_holds_every_marker_of_the_substring_rule() {
        for marker in ORACLE_MARKERS {
            let term = marker.trim_end().as_bytes();
            assert!(SENSITIVE_TERMS.contains(&term), "{marker:?}");
        }
        for term in SENSITIVE_TERMS {
            assert!(sensitive_word(term));
        }
    }
    #[test]
    fn arbitrary_lines_the_substring_rule_redacts_are_redacted_unless_plain_prose() {
        let words: Vec<String> = SENSITIVE_TERMS
            .iter()
            .map(|term| std::str::from_utf8(term).unwrap())
            .chain(["api__key", "private.key", "API2KEY", "Bearer"])
            .flat_map(|term| {
                ["", "my", "db", "obsession"]
                    .into_iter()
                    .flat_map(move |lead| {
                        ["", "s", "file", "value", "izer", "ary", "managementx"]
                            .into_iter()
                            .flat_map(move |tail| cased(&format!("{lead}{term}{tail}")))
                    })
            })
            .collect();
        // No piece makes a `sk-` part on its own: the oracle's value shape,
        // which the shape rule deliberately narrows (`sk-learn-compatible`).
        let pieces: &[&str] = &[
            "alpha",
            "bravo",
            "the",
            "is",
            "a",
            "user",
            "bob",
            "Bob",
            "BOB",
            "42",
            "x9",
            ".",
            ",",
            ";",
            "!",
            "?",
            "=",
            ":",
            "-",
            "_",
            "\"",
            "'",
            "/",
            "@",
            "://",
            "{",
            "}",
            "(",
            ")",
            "[",
            "]",
            "\t",
            "é",
            "😀",
            "%3D",
            "abcdefghijklmnop",
            "hunter2",
            "HuNter2!",
        ];
        let mut next = generator(0x5851_f42d_4c95_7f2d);
        let (mut shown, mut hidden) = (0, 0);
        for _ in 0..20_000 {
            let mut line = String::new();
            for _ in 0..1 + next(12) {
                if !line.is_empty() && next(3) != 0 {
                    line.push(' ');
                }
                if next(3) == 0 {
                    line.push_str(&words[next(words.len())]);
                } else {
                    line.push_str(pieces[next(pieces.len())]);
                }
            }
            let prose = prose_definition(&line);
            // The scan decides exactly as the definition does.
            assert_eq!(
                named_without_prose(line.as_bytes()),
                prose == Some(false),
                "{line:?}"
            );
            if substring_rule(&line) {
                if redacted(&line) {
                    hidden += 1;
                } else {
                    assert_eq!(prose, Some(true), "{line:?}");
                    shown += 1;
                }
            }
        }
        // Both outcomes occur often, so neither half is vacuous.
        assert!(shown >= 100 && hidden >= 1000, "{shown} {hidden}");
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
