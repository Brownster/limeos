use super::*;

/// An endless source that records how many bytes were ever handed out.
struct Endless<'a> {
    served: &'a std::cell::Cell<u64>,
}

impl Read for Endless<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(0);
        self.served.set(self.served.get() + buf.len() as u64);
        Ok(buf.len())
    }
}

fn limited<'a, R: Read>(inner: R, limit: u64, cancelled: &'a dyn Fn() -> bool) -> Limited<'a, R> {
    Limited {
        inner,
        count: 0,
        limit,
        reason: Stop::Decompressed,
        hasher: None,
        cancelled,
    }
}

#[test]
fn reads_stop_one_byte_past_the_limit_of_an_endless_source() {
    let served = std::cell::Cell::new(0);
    let never = || false;
    let mut reader = limited(Endless { served: &served }, 100_000, &never);
    let error = io::copy(&mut reader, &mut io::sink()).unwrap_err();
    assert_eq!(stop_of(&error), Some(Stop::Decompressed));
    assert_eq!(
        served.get(),
        100_001,
        "never more than limit + 1 bytes are requested"
    );
}

#[test]
fn exactly_the_limit_is_accepted_and_cancellation_interrupts() {
    let never = || false;
    let mut exact = limited(&[7u8; 4096][..], 4096, &never);
    assert_eq!(io::copy(&mut exact, &mut io::sink()).unwrap(), 4096);
    let always = || true;
    let mut cancelled = limited(&[7u8; 16][..], 4096, &always);
    let error = cancelled.read(&mut [0; 8]).unwrap_err();
    assert_eq!(classify(&error, None).codes(), [FindingCode::Cancelled]);
}

#[test]
fn stop_reasons_survive_wrapping_by_decoders_and_parsers() {
    let wrapped = io::Error::other(io::Error::other(Stop::Compressed));
    assert_eq!(
        classify(&wrapped, Some(3)).findings[0].code,
        FindingCode::CompressedLimit
    );
    assert_eq!(classify(&wrapped, Some(3)).findings[0].entry, Some(3));
    let eof = io::Error::from(io::ErrorKind::UnexpectedEof);
    assert_eq!(classify(&eof, None).codes(), [FindingCode::Truncated]);
}
