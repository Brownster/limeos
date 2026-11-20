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
/// keep-alive proves the loop is still being run, and nothing more: a
/// service that is alive but degraded keeps sending it and reports the
/// degradation through its health answer, since a restart cannot fix what
/// a stuck device or a stuck kernel read has broken. Does nothing unless
/// systemd set `NOTIFY_SOCKET` and `WATCHDOG_USEC` for this process
/// (`WatchdogSec=`).
pub struct Watchdog {
    target: Option<SocketAddr>,
    interval: Duration,
    next: tokio::time::Instant,
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
        }
    }
    pub fn disabled() -> Self {
        Self {
            target: None,
            interval: Duration::MAX,
            next: tokio::time::Instant::now(),
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
    /// Sends `WATCHDOG=1` and schedules the next one.
    pub fn beat(&mut self) -> std::io::Result<()> {
        let Some(target) = &self.target else {
            return Ok(());
        };
        self.next = tokio::time::Instant::now() + self.interval;
        notify(target, "WATCHDOG=1")
    }
}
/// systemd's notification socket for this service, if it set one.
pub fn notify_socket() -> Option<SocketAddr> {
    notify_address(&std::env::var_os("NOTIFY_SOCKET")?)
}

/// Tells systemd (`Type=notify`) that the service is ready to serve. Its
/// watchdog clock starts here, not at exec. Nothing without a notification
/// socket.
pub fn notify_ready() -> std::io::Result<()> {
    match notify_socket() {
        Some(target) => notify(&target, "READY=1\nSTATUS=Serving"),
        None => Ok(()),
    }
}

/// While held, keeps extending systemd's start timeout for a startup step
/// whose duration cannot be bounded in advance (a schema upgrade), so the
/// service is not killed and rolled back mid-step and started again only
/// to repeat it. Every `every` it asks for `extend` more, with `status` for
/// `systemctl status`, until `cap` has passed: past that the step is not
/// slow but stuck (on a failing device, say), and the start timeout is let
/// to fail the start where an operator sees it.
pub struct StartExtension {
    stop: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl StartExtension {
    /// One minute at a time, renewed every 20 s, for at most 30 minutes.
    pub fn from_env(status: &str) -> Self {
        Self::new(
            notify_socket(),
            status,
            Duration::from_secs(20),
            Duration::from_secs(60),
            Duration::from_secs(30 * 60),
        )
    }
    pub fn new(
        target: Option<SocketAddr>,
        status: &str,
        every: Duration,
        extend: Duration,
        cap: Duration,
    ) -> Self {
        let Some(target) = target else {
            return Self {
                stop: None,
                thread: None,
            };
        };
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let message = format!(
            "EXTEND_TIMEOUT_USEC={}\nSTATUS={status}",
            extend.as_micros()
        );
        let thread = std::thread::Builder::new()
            .name("start-extension".into())
            .spawn(move || {
                let until = std::time::Instant::now() + cap;
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                    stopped.recv_timeout(every)
                {
                    if std::time::Instant::now() >= until {
                        return;
                    }
                    let _ = notify(&target, &message);
                }
            })
            .ok();
        Self {
            stop: Some(stop),
            thread,
        }
    }
}
impl Drop for StartExtension {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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
    async fn keep_alives_reach_an_abstract_socket_on_schedule() {
        use std::os::linux::net::SocketAddrExt;
        let name = format!("limeos-watchdog-test-{}", std::process::id());
        let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let receiver = UnixDatagram::bind_addr(&address).unwrap();
        receiver.set_nonblocking(true).unwrap();
        let target = notify_address(OsStr::new(&format!("@{name}"))).unwrap();
        let mut watchdog = Watchdog::new(target, Duration::from_millis(200));
        watchdog.due().await;
        watchdog.beat().unwrap();
        let mut buffer = [0; 32];
        let length = receiver.recv(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"WATCHDOG=1");
        // The next one is due an interval later, not at once.
        assert!(
            tokio::time::timeout(Duration::from_millis(20), watchdog.due())
                .await
                .is_err()
        );
        watchdog.due().await;
        watchdog.beat().unwrap();
        assert!(receiver.recv(&mut buffer).is_ok());
        let disabled = Watchdog::disabled();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), disabled.due())
                .await
                .is_err()
        );
    }

    #[test]
    fn a_start_extension_renews_while_held_stops_when_dropped_and_at_its_cap() {
        use std::os::linux::net::SocketAddrExt;
        let name = format!("limeos-extension-test-{}", std::process::id());
        let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let receiver = UnixDatagram::bind_addr(&address).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let target = notify_address(OsStr::new(&format!("@{name}"))).unwrap();
        let mut buffer = [0; 128];
        {
            let _held = StartExtension::new(
                Some(target.clone()),
                "Upgrading",
                Duration::from_millis(20),
                Duration::from_secs(60),
                Duration::from_secs(60),
            );
            for _ in 0..2 {
                let length = receiver.recv(&mut buffer).unwrap();
                assert_eq!(
                    &buffer[..length],
                    b"EXTEND_TIMEOUT_USEC=60000000\nSTATUS=Upgrading"
                );
            }
        }
        // Dropped: nothing more arrives.
        receiver.set_nonblocking(true).unwrap();
        while receiver.recv(&mut buffer).is_ok() {}
        std::thread::sleep(Duration::from_millis(100));
        assert!(receiver.recv(&mut buffer).is_err());
        // Past its cap it stops on its own, held or not.
        let held = StartExtension::new(
            Some(target),
            "Upgrading",
            Duration::from_millis(10),
            Duration::from_secs(60),
            Duration::from_millis(50),
        );
        std::thread::sleep(Duration::from_millis(200));
        while receiver.recv(&mut buffer).is_ok() {}
        std::thread::sleep(Duration::from_millis(100));
        assert!(receiver.recv(&mut buffer).is_err());
        drop(held);
        // Without a notification socket it does nothing at all.
        drop(StartExtension::new(
            None,
            "x",
            Duration::from_millis(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ));
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
