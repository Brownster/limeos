//! Helpers shared by the resident services' accept loops.
use std::{
    ffi::OsStr,
    os::unix::net::{SocketAddr, UnixDatagram},
    time::Duration,
};

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

/// The systemd watchdog, sent from inside a service's accept loop so a
/// keep-alive proves the loop is still being run. Does nothing unless systemd
/// set `NOTIFY_SOCKET` and `WATCHDOG_USEC` for this process (`WatchdogSec=`).
pub struct Watchdog {
    target: Option<SocketAddr>,
    interval: Duration,
    next: tokio::time::Instant,
    withholding: bool,
}
/// What one due keep-alive did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Beat {
    Sent,
    /// Sent again after being withheld.
    Resumed,
    /// Withheld because the service is unhealthy; systemd restarts it if
    /// this lasts `WatchdogSec`.
    Withheld,
    StillWithheld,
}
impl Watchdog {
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var_os("NOTIFY_SOCKET").as_deref(),
            std::env::var("WATCHDOG_USEC").ok().as_deref(),
            std::env::var("WATCHDOG_PID").ok().as_deref(),
            std::process::id(),
        )
    }
    fn from_values(
        socket: Option<&OsStr>,
        usec: Option<&str>,
        pid: Option<&str>,
        own_pid: u32,
    ) -> Self {
        let usec = usec.and_then(|v| v.parse::<u64>().ok()).filter(|v| *v > 0);
        // An inherited variable names another process (the service's main
        // process); only that process may answer for the service.
        let ours = pid.is_none_or(|pid| pid.parse::<u32>().ok() == Some(own_pid));
        match (socket.and_then(notify_address), usec) {
            // Three keep-alives per watchdog period, as systemd recommends
            // at least two.
            (Some(target), Some(usec)) if ours => {
                Self::new(target, Duration::from_micros(usec) / 3)
            }
            _ => Self::disabled(),
        }
    }
    pub fn new(target: SocketAddr, interval: Duration) -> Self {
        Self {
            target: Some(target),
            interval: interval.max(Duration::from_millis(10)),
            next: tokio::time::Instant::now(),
            withholding: false,
        }
    }
    pub fn disabled() -> Self {
        Self {
            target: None,
            interval: Duration::MAX,
            next: tokio::time::Instant::now(),
            withholding: false,
        }
    }
    pub fn enabled(&self) -> bool {
        self.target.is_some()
    }
    /// Completes when a keep-alive is due; never, when disabled.
    pub async fn due(&self) {
        if self.target.is_some() {
            tokio::time::sleep_until(self.next).await;
        } else {
            std::future::pending::<()>().await;
        }
    }
    /// Sends `WATCHDOG=1` if `healthy`, and schedules the next check either way.
    pub fn beat(&mut self, healthy: bool) -> std::io::Result<Beat> {
        let Some(target) = &self.target else {
            return Ok(Beat::StillWithheld);
        };
        self.next = tokio::time::Instant::now() + self.interval;
        if !healthy {
            let first = !self.withholding;
            self.withholding = true;
            return Ok(if first {
                Beat::Withheld
            } else {
                Beat::StillWithheld
            });
        }
        notify(target, "WATCHDOG=1")?;
        Ok(if std::mem::take(&mut self.withholding) {
            Beat::Resumed
        } else {
            Beat::Sent
        })
    }
}
/// `NOTIFY_SOCKET` names a filesystem path, or an abstract socket with `@`.
fn notify_address(value: &OsStr) -> Option<SocketAddr> {
    use std::os::{linux::net::SocketAddrExt, unix::ffi::OsStrExt};
    match value.as_bytes().split_first() {
        Some((b'/', _)) => SocketAddr::from_pathname(value).ok(),
        Some((b'@', name)) => SocketAddr::from_abstract_name(name).ok(),
        _ => None,
    }
}
/// One datagram to systemd's notification socket; never blocks.
pub fn notify(target: &SocketAddr, state: &str) -> std::io::Result<()> {
    let socket = UnixDatagram::unbound()?;
    socket.set_nonblocking(true)?;
    socket.send_to_addr(state.as_bytes(), target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watchdog_is_enabled_only_for_the_process_systemd_named() {
        let path = Some(OsStr::new("/run/systemd/notify"));
        let ours = Watchdog::from_values(path, Some("30000000"), Some("42"), 42);
        assert!(ours.enabled());
        assert_eq!(ours.interval, Duration::from_secs(10));
        assert!(Watchdog::from_values(path, Some("30000000"), None, 42).enabled());
        for (socket, usec, pid) in [
            (None, Some("30000000"), None),
            (path, None, None),
            (path, Some("0"), None),
            (path, Some("x"), None),
            (path, Some("30000000"), Some("41")),
            (Some(OsStr::new("relative")), Some("30000000"), None),
            (Some(OsStr::new("")), Some("30000000"), None),
        ] {
            assert!(!Watchdog::from_values(socket, usec, pid, 42).enabled());
        }
        assert!(Watchdog::from_values(Some(OsStr::new("@abstract")), Some("1"), None, 1).enabled());
    }

    #[tokio::test]
    async fn keep_alives_reach_an_abstract_socket_and_stop_while_unhealthy() {
        use std::os::linux::net::SocketAddrExt;
        let name = format!("limeos-watchdog-test-{}", std::process::id());
        let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let receiver = UnixDatagram::bind_addr(&address).unwrap();
        receiver.set_nonblocking(true).unwrap();
        let target = notify_address(OsStr::new(&format!("@{name}"))).unwrap();
        let mut watchdog = Watchdog::new(target, Duration::from_millis(10));
        watchdog.due().await;
        assert_eq!(watchdog.beat(true).unwrap(), Beat::Sent);
        let mut buffer = [0; 32];
        let length = receiver.recv(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"WATCHDOG=1");
        assert_eq!(watchdog.beat(false).unwrap(), Beat::Withheld);
        assert_eq!(watchdog.beat(false).unwrap(), Beat::StillWithheld);
        assert!(receiver.recv(&mut buffer).is_err());
        assert_eq!(watchdog.beat(true).unwrap(), Beat::Resumed);
        assert!(receiver.recv(&mut buffer).is_ok());
        let disabled = Watchdog::disabled();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), disabled.due())
                .await
                .is_err()
        );
    }

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
