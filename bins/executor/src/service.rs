//! The resident executor roles' accept loop. Errors that leave the listener
//! usable pause and continue; the systemd watchdog keep-alive is sent from
//! here, so it stops only when the loop stops being run. A role that is
//! alive but degraded keeps sending it and says so in its health answer.
use crate::blocking_read::BlockingReads;
use limeos_contracts::{ACCEPT_PAUSE, Watchdog, accept_retryable};
use limeos_executor_protocol::{Receipt, StrandedReads};
use tokio::net::{UnixListener, UnixStream};

/// Waits for the next connection, sending due keep-alives meanwhile.
pub async fn next_connection(
    listener: &UnixListener,
    watchdog: &mut Watchdog,
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
            () = watchdog.due() => {
                if let Err(error) = watchdog.beat() {
                    tracing::warn!(%error, "watchdog keep-alive could not be sent");
                }
            }
        }
    }
}

/// The storage reader's answer to `Health`: ready, with any reads still
/// stuck past their deadline and since when, for core to report.
pub fn health_answer(receipt: &mut Receipt, reads: &BlockingReads) {
    receipt.stranded_reads = reads.stranded().map(|stranded| StrandedReads {
        count: u32::try_from(stranded.count).unwrap_or(u32::MAX),
        since: stranded.since,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocking_read::Refusal;
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

    /// A storage read that never returns is abandoned at its deadline. The
    /// service keeps answering other requests and keeps sending its
    /// keep-alive (a restart would not free the read), refuses further reads,
    /// and reports the stuck read and since when in its health answer until
    /// the read returns.
    #[tokio::test]
    async fn a_hung_read_is_reported_and_refused_while_the_service_and_its_keep_alive_go_on() {
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
                let mut stream = next_connection(&listener, &mut watchdog).await.unwrap();
                let reads = served.clone();
                let blocked = blocked.clone();
                tokio::spawn(async move {
                    let mut command = [0];
                    stream.read_exact(&mut command).await.unwrap();
                    let answer: Vec<u8> = match command[0] {
                        b'r' => {
                            let read = blocked.lock().unwrap().take();
                            match reads
                                .run("test-read", Duration::from_millis(50), move || {
                                    read.map(|r| r.recv())
                                })
                                .await
                            {
                                Ok(_) => b"read".to_vec(),
                                Err(Refusal::Deadline) => b"deadline".to_vec(),
                                Err(Refusal::Stranded) => b"stranded".to_vec(),
                                Err(Refusal::Unavailable) => b"unavailable".to_vec(),
                            }
                        }
                        _ => {
                            let mut receipt = Receipt::empty();
                            health_answer(&mut receipt, &reads);
                            serde_json::to_vec(&receipt.stranded_reads).unwrap()
                        }
                    };
                    stream.write_all(&answer).await.unwrap();
                });
            }
        });
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(beats(&notify) >= 2, "no keep-alives while healthy");
        assert_eq!(ask(&path, b'h').await, b"null");

        let abandoned = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(ask(&path, b'r').await, b"deadline");
        let reported: Option<StrandedReads> =
            serde_json::from_slice(&ask(&path, b'h').await).unwrap();
        let reported = reported.expect("the stuck read is reported");
        assert_eq!(reported.count, 1);
        assert!((reported.since - abandoned).abs() <= 2, "{reported:?}");
        assert_eq!(ask(&path, b'r').await, b"stranded");
        // Keep-alives go on while the read is stuck.
        beats(&notify);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(beats(&notify) >= 2, "keep-alive withheld with a stuck read");

        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while ask(&path, b'h').await != b"null" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the report clears once the read returns");
        service.abort();
    }
}
