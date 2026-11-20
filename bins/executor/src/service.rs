//! The resident executor roles' accept loop. Errors that leave the listener
//! usable pause and continue; the systemd watchdog keep-alive is sent from
//! here, so it stops when the loop stops being run or the role reports
//! itself unhealthy.
use limeos_contracts::{ACCEPT_PAUSE, Beat, Watchdog, accept_retryable};
use tokio::net::{UnixListener, UnixStream};

/// Waits for the next connection, sending due keep-alives meanwhile.
pub async fn next_connection(
    listener: &UnixListener,
    watchdog: &mut Watchdog,
    healthy: &(dyn Fn() -> bool + Sync),
) -> std::io::Result<UnixStream> {
    let mut failures = 0u64;
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => return Ok(stream),
                Err(error) if accept_retryable(&error) => {
                    failures += 1;
                    if failures.is_power_of_two() {
                        tracing::warn!(%error, failures, "executor accept failed; retrying");
                    }
                    tokio::time::sleep(ACCEPT_PAUSE).await;
                }
                Err(error) => return Err(error),
            },
            () = watchdog.due() => match watchdog.beat(healthy()) {
                Ok(Beat::Withheld) => tracing::error!(
                    "executor unhealthy; withholding the watchdog keep-alive so systemd restarts it unless it recovers"
                ),
                Ok(Beat::Resumed) => tracing::info!("executor healthy again; watchdog keep-alive resumed"),
                Ok(Beat::Sent | Beat::StillWithheld) => {}
                Err(error) => tracing::warn!(%error, "watchdog keep-alive could not be sent"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocking_read::{BlockingReads, Refusal};
    use std::{os::unix::net::UnixDatagram, sync::Arc, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("limeos-service-{}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    async fn ask(path: &std::path::Path, command: u8) -> Vec<u8> {
        let mut stream = UnixStream::connect(path).await.unwrap();
        stream.write_all(&[command]).await.unwrap();
        let mut answer = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut answer))
            .await
            .unwrap()
            .unwrap();
        answer
    }
    fn beats(notify: &UnixDatagram) -> usize {
        let mut buffer = [0; 16];
        std::iter::from_fn(|| notify.recv(&mut buffer).ok()).count()
    }

    /// A storage read that never returns is abandoned at its deadline; the
    /// service keeps answering other requests, and the keep-alive stops only
    /// once the stranded-read bound is reached, resuming when it clears.
    #[tokio::test]
    async fn a_hung_read_does_not_stop_the_service_and_withholds_the_keep_alive_only_at_the_bound()
    {
        let dir = Directory::new();
        let notify = UnixDatagram::bind(dir.0.join("notify")).unwrap();
        notify.set_nonblocking(true).unwrap();
        let path = dir.0.join("executor.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let reads = Arc::new(BlockingReads::new(1));
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let blocked = Arc::new(std::sync::Mutex::new(Some(blocked)));
        let mut watchdog = Watchdog::new(
            std::os::unix::net::SocketAddr::from_pathname(dir.0.join("notify")).unwrap(),
            Duration::from_millis(10),
        );
        let served = reads.clone();
        let service = tokio::spawn(async move {
            loop {
                let check = served.clone();
                let mut stream =
                    next_connection(&listener, &mut watchdog, &move || check.healthy())
                        .await
                        .unwrap();
                let reads = served.clone();
                let blocked = blocked.clone();
                tokio::spawn(async move {
                    let mut command = [0];
                    stream.read_exact(&mut command).await.unwrap();
                    let answer: &[u8] = if command[0] == b'r' {
                        let read = blocked.lock().unwrap().take();
                        match reads
                            .run("test-read", Duration::from_millis(50), move || {
                                read.map(|r| r.recv())
                            })
                            .await
                        {
                            Ok(_) => b"read",
                            Err(Refusal::Deadline) => b"deadline",
                            Err(Refusal::Stranded) => b"stranded",
                            Err(Refusal::Unavailable) => b"unavailable",
                        }
                    } else {
                        b"ok"
                    };
                    stream.write_all(answer).await.unwrap();
                });
            }
        });
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(beats(&notify) >= 2, "no keep-alives while healthy");

        assert_eq!(ask(&path, b'r').await, b"deadline");
        assert!(!reads.healthy());
        beats(&notify);
        // Still answering while the read is stuck, and no new thread starts.
        assert_eq!(ask(&path, b'h').await, b"ok");
        assert_eq!(ask(&path, b'r').await, b"stranded");
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(beats(&notify), 0, "keep-alive sent with a stranded read");

        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while beats(&notify) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("keep-alives resume once the read returns");
        assert_eq!(ask(&path, b'h').await, b"ok");
        service.abort();
    }
}
