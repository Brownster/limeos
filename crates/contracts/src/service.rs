//! Helpers shared by the resident services' accept loops.
use std::time::Duration;

/// How long an accept loop waits after a failure it can retry, so descriptor
/// exhaustion does not become a busy loop.
pub const ACCEPT_PAUSE: Duration = Duration::from_millis(250);

/// Whether a failed `accept` leaves the listener usable: the failure belongs
/// to one aborted connection or to a momentary shortage of descriptors,
/// memory or buffers. Anything else (a closed or invalid listener) is fatal.
pub fn accept_retryable(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    // Linux errno values (identical on x86_64 and arm64): EPERM (a firewall
    // refused the connection), ENOMEM, ENFILE, EMFILE, EPROTO, ENOBUFS, and
    // the network errors accept(2) says to treat like EAGAIN.
    const RETRYABLE: [i32; 14] = [1, 12, 23, 24, 64, 71, 92, 95, 100, 101, 103, 105, 112, 113];
    matches!(
        error.kind(),
        ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::Interrupted
            | ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::OutOfMemory
    ) || error
        .raw_os_error()
        .is_some_and(|code| RETRYABLE.contains(&code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_exhaustion_and_aborted_peers_are_retried_but_a_broken_listener_is_not() {
        for code in [24, 23, 103, 105, 12, 71, 4] {
            assert!(
                accept_retryable(&std::io::Error::from_raw_os_error(code)),
                "{code}"
            );
        }
        // EBADF, EINVAL, ENOTSOCK: the listener itself is unusable.
        for code in [9, 22, 88] {
            assert!(
                !accept_retryable(&std::io::Error::from_raw_os_error(code)),
                "{code}"
            );
        }
    }
}
