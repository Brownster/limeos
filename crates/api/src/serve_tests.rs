//! `serve` over a real socket, with the limits shortened so the tests are
//! fast; each test says which default it stands for.
use super::*;
use axum::{
    body::{Body, Bytes},
    extract::Path,
};
use std::net::SocketAddr;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    time::Instant,
};

/// The defaults scaled down: 5 s header and body become 100 ms, the 30 s
/// idle gap 300 ms.
const SHORT: Limits = Limits {
    connections: 64,
    header: Duration::from_millis(100),
    body: Duration::from_millis(100),
    idle: Duration::from_millis(300),
    response: Duration::from_secs(5),
};
async fn start(router: Router, limits: Limits) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(serve_with(listener, router, limits));
    address
}
fn router() -> Router {
    Router::new()
        .route("/fast", get(|| async { "fast" }))
        .route(
            "/sleep/{ms}",
            get(|Path(ms): Path<u64>| async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                "slept"
            }),
        )
        .route("/echo", post(|body: Bytes| async move { body }))
        .route("/large", get(|| async { "x".repeat(LARGE) }))
        .route(
            "/ignore",
            post(|body: Body| async move {
                // Answered at once; the body is kept but never read.
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    drop(body);
                });
                "x".repeat(HUGE)
            }),
        )
        .route("/endless", get(|| async { ticks(u64::MAX, 50) }))
        .route(
            "/stream/{ms}",
            get(|Path(ms): Path<u64>| async move { ticks(ms / 100, 100) }),
        )
        .layer(middleware::from_fn(limits))
}
/// A streamed (chunked) body of `count` lines, one every `every` ms.
fn ticks(count: u64, every: u64) -> Body {
    let lines = futures_util::stream::unfold(0, move |sent| async move {
        if sent == count {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(every)).await;
        Some((Ok::<_, std::convert::Infallible>("tick\n"), sent + 1))
    });
    Body::from_stream(lines)
}
/// Below hyper's 16 KiB write buffer, so hyper takes the whole body and is
/// done with it before any of it has been sent.
const LARGE: usize = 15_000;
fn request_line(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nhost: localhost\r\n\r\n")
}
struct Client(BufReader<TcpStream>);
impl Client {
    async fn connect(address: SocketAddr) -> Self {
        Self(BufReader::new(TcpStream::connect(address).await.unwrap()))
    }
    async fn send(&mut self, raw: &str) {
        self.0.get_mut().write_all(raw.as_bytes()).await.unwrap();
    }
    /// The next response's status and body, or `None` once the server closed.
    async fn response(&mut self) -> Option<(u16, String)> {
        let mut line = String::new();
        if self.0.read_line(&mut line).await.unwrap_or(0) == 0 {
            return None;
        }
        let status = line.split(' ').nth(1)?.parse().ok()?;
        let mut length = 0;
        let mut chunked = false;
        loop {
            line.clear();
            self.0.read_line(&mut line).await.ok()?;
            let header = line.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().ok()?;
                }
                if name.eq_ignore_ascii_case("transfer-encoding") {
                    chunked = value.trim().eq_ignore_ascii_case("chunked");
                }
            }
        }
        if !chunked {
            let mut body = vec![0; length];
            self.0.read_exact(&mut body).await.ok()?;
            return Some((status, String::from_utf8(body).ok()?));
        }
        let mut body = Vec::new();
        loop {
            line.clear();
            self.0.read_line(&mut line).await.ok()?;
            let size = usize::from_str_radix(line.trim_end(), 16).ok()?;
            let mut chunk = vec![0; size + 2];
            self.0.read_exact(&mut chunk).await.ok()?;
            if size == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..size]);
        }
        Some((status, String::from_utf8(body).ok()?))
    }
    async fn closed(&mut self) -> bool {
        let mut byte = [0];
        matches!(self.0.read(&mut byte).await, Ok(0) | Err(_))
    }
}

/// The defaults scaled by 1/25 with wide margins: header and body 5 s become
/// 200 ms, idle 30 s 1.2 s, response 60 s 2.4 s and the 25 s stream lifetime
/// 1 s. The fixed whole-connection limit this replaced was 30 s, the idle
/// limit: 1.2 s at this scale.
const SCALED: Limits = Limits {
    connections: 64,
    header: Duration::from_millis(200),
    body: Duration::from_millis(200),
    idle: Duration::from_millis(1200),
    response: Duration::from_millis(2400),
};
const OLD_LIFETIME: Duration = Duration::from_millis(1200);

/// A keep-alive connection from the proxy, used for five times the old
/// connection lifetime, with a request in flight across the point where
/// that lifetime would have cut it off; every request is answered.
#[tokio::test]
async fn a_keep_alive_connection_in_use_is_served_far_past_the_old_connection_lifetime() {
    let address = start(router(), SCALED).await;
    let started = Instant::now();
    let mut client = Client::connect(address).await;
    client.send(&request_line("/fast")).await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
    // In flight from about 0.6 s to 2.4 s, across the old 1.2 s lifetime:
    // longer than the idle limit, 600 ms inside the response limit.
    tokio::time::sleep(Duration::from_millis(600)).await;
    client.send(&request_line("/sleep/1800")).await;
    assert_eq!(client.response().await, Some((200, "slept".into())));
    // Then gaps of half the idle limit (three header limits each) until
    // five old lifetimes have passed.
    let mut served = 2;
    while started.elapsed() < OLD_LIFETIME * 5 {
        tokio::time::sleep(Duration::from_millis(600)).await;
        client.send(&request_line("/fast")).await;
        assert_eq!(
            client.response().await,
            Some((200, "fast".into())),
            "request {served} at {:?}",
            started.elapsed()
        );
        served += 1;
    }
    assert!(served >= 8, "{served}");
}

#[tokio::test]
async fn an_idle_keep_alive_connection_stays_open_for_the_idle_limit_and_then_closes() {
    let address = start(router(), SCALED).await;
    let mut client = Client::connect(address).await;
    client.send(&request_line("/fast")).await;
    assert!(client.response().await.is_some());
    // Three header limits: idle time is not header time.
    tokio::time::sleep(Duration::from_millis(600)).await;
    client.send(&request_line("/fast")).await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
    let idle = Instant::now();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), client.closed())
            .await
            .unwrap()
    );
    let waited = idle.elapsed();
    assert!(
        waited >= Duration::from_millis(1000) && waited < Duration::from_secs(4),
        "{waited:?}"
    );
}

/// A keep-alive connection whose second request's headers stall: dropped at
/// the header limit counted from that request's first byte, far inside the
/// idle limit it waited under before.
#[tokio::test]
async fn slow_headers_on_a_second_keep_alive_request_are_dropped_at_the_header_limit() {
    let address = start(router(), SCALED).await;
    let mut client = Client::connect(address).await;
    client.send(&request_line("/fast")).await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
    tokio::time::sleep(Duration::from_millis(600)).await;
    let started = Instant::now();
    client.send("GET /fast HTTP/1.1\r\nhost: loc").await;
    assert!(
        tokio::time::timeout(Duration::from_secs(10), client.closed())
            .await
            .unwrap()
    );
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(150) && waited < Duration::from_millis(900),
        "{waited:?}"
    );
}

/// Reads everything until the server closes, returning it and how long that
/// took.
async fn read_until_closed(client: &mut Client) -> (Vec<u8>, Duration) {
    let started = Instant::now();
    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut chunk = [0; 4096];
        while let Ok(n @ 1..) = client.0.read(&mut chunk).await {
            received.extend_from_slice(&chunk[..n]);
        }
    })
    .await
    .unwrap();
    (received, started.elapsed())
}

/// A response that never ends, read promptly, is cut off at the response
/// limit: neither the idle nor the header limit ends it, and nothing keeps
/// it open longer.
#[tokio::test]
async fn an_endless_response_is_cut_off_at_the_response_limit() {
    let address = start(router(), SCALED).await;
    let mut client = Client::connect(address).await;
    client.send(&request_line("/endless")).await;
    let (received, waited) = read_until_closed(&mut client).await;
    assert!(
        waited >= Duration::from_millis(2200) && waited < Duration::from_secs(5),
        "{waited:?}"
    );
    // Chunks kept arriving all along: about one per 50 ms.
    let chunks = received.windows(5).filter(|w| w == b"tick\n").count();
    assert!(chunks >= 30, "{chunks}");
}

/// An event stream is not cut by the response limit: it ends itself at its
/// lifetime, well inside that limit, and the connection then carries the
/// next request.
#[tokio::test]
async fn an_event_stream_runs_to_its_lifetime_inside_the_response_limit() {
    const _: () =
        assert!(crate::reads::STREAM_LIFETIME.as_secs() * 2 <= Limits::DEFAULT_RESPONSE.as_secs());
    let address = start(router(), SCALED).await;
    let mut client = Client::connect(address).await;
    // The 25 s lifetime at scale, with a tick every 100 ms (the stream's
    // longest gap is 5 s, beyond the 1.2 s idle limit at this scale).
    client.send(&request_line("/stream/1000")).await;
    let (status, body) = client.response().await.unwrap();
    assert_eq!((status, body.matches("tick").count()), (200, 10));
    client.send(&request_line("/fast")).await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
}

#[tokio::test]
async fn a_client_that_never_finishes_its_headers_is_dropped_at_the_header_limit() {
    let address = start(router(), SHORT).await;
    let mut client = Client::connect(address).await;
    let started = Instant::now();
    client.send("GET /fast HTTP/1.1\r\nhost: loc").await;
    assert!(client.closed().await);
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(90) && waited < Duration::from_secs(3),
        "{waited:?}"
    );
}

/// A request coalesced with the start of the next one: hyper buffers the
/// second request's first bytes while it serves the first, so no further
/// socket read marks them. Its headers still get the 5 s header limit, not
/// the 30 s idle limit, counted from the end of the first response.
#[tokio::test]
async fn a_pipelined_request_that_never_finishes_its_headers_is_dropped_at_the_header_limit() {
    // Header 5 s and idle 30 s scaled to 200 ms and 3 s.
    let limits = Limits {
        header: Duration::from_millis(200),
        idle: Duration::from_secs(3),
        ..SHORT
    };
    let address = start(router(), limits).await;
    let mut client = Client::connect(address).await;
    client
        .send(&format!(
            "{}GET /fast HTTP/1.1\r\nhost: loc",
            request_line("/fast")
        ))
        .await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
    let answered = Instant::now();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), client.closed())
            .await
            .unwrap()
    );
    let waited = answered.elapsed();
    assert!(
        waited >= Duration::from_millis(150) && waited < Duration::from_millis(1500),
        "{waited:?}"
    );
}

/// The same after a request with a body: the body's length on the wire,
/// not the socket reads, says where the next request starts.
#[tokio::test]
async fn a_pipelined_request_after_a_body_gets_the_header_limit() {
    let limits = Limits {
        header: Duration::from_millis(200),
        idle: Duration::from_secs(3),
        ..SHORT
    };
    let address = start(router(), limits).await;
    let mut client = Client::connect(address).await;
    client
        .send("POST /echo HTTP/1.1\r\nhost: localhost\r\ncontent-length: 8\r\n\r\nab\r\n\r\ncdGET /fast HTTP/1.1\r\nho")
        .await;
    assert_eq!(client.response().await, Some((200, "ab\r\n\r\ncd".into())));
    let answered = Instant::now();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), client.closed())
            .await
            .unwrap()
    );
    assert!(
        answered.elapsed() < Duration::from_millis(1500),
        "{:?}",
        answered.elapsed()
    );
}

/// A chunked body's end is not tracked, so its answer closes the connection
/// rather than reading a next request whose start is unknown.
#[tokio::test]
async fn a_chunked_request_is_answered_and_its_connection_closed() {
    let address = start(router(), SHORT).await;
    let mut client = Client::connect(address).await;
    client
        .send("POST /echo HTTP/1.1\r\nhost: localhost\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n")
        .await;
    assert_eq!(client.response().await, Some((200, "abc".into())));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), client.closed())
            .await
            .unwrap()
    );
}

/// Blank lines before a request line are skipped, as hyper skips them; a
/// request split across many writes is still one request.
#[tokio::test]
async fn leading_blank_lines_and_split_headers_are_one_request() {
    let address = start(router(), SHORT).await;
    let mut client = Client::connect(address).await;
    for part in [
        "\r\n\r\nGET /fa",
        "st HTTP/1.1\r",
        "\nhost: localhost\r\n",
        "\r\n",
    ] {
        client.send(part).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(client.response().await, Some((200, "fast".into())));
    client.send("GET /fast HTTP/1.1\nhost: localhost\n\n").await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
}

/// A complete pipelined request is answered, and the connection then idles
/// under the idle limit as usual.
#[tokio::test]
async fn complete_pipelined_requests_are_all_answered() {
    let limits = Limits {
        header: Duration::from_millis(200),
        idle: Duration::from_secs(3),
        ..SHORT
    };
    let address = start(router(), limits).await;
    let mut client = Client::connect(address).await;
    client
        .send(&format!(
            "{}{}",
            request_line("/fast"),
            request_line("/sleep/10")
        ))
        .await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
    assert_eq!(client.response().await, Some((200, "slept".into())));
    // Idle well past the header limit, then served again.
    tokio::time::sleep(Duration::from_millis(800)).await;
    client.send(&request_line("/fast")).await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
}

#[tokio::test]
async fn a_client_that_never_finishes_its_body_is_dropped_at_the_body_limit() {
    let address = start(router(), SHORT).await;
    let mut client = Client::connect(address).await;
    let started = Instant::now();
    client
        .send("POST /echo HTTP/1.1\r\nhost: localhost\r\ncontent-length: 10\r\n\r\nabc")
        .await;
    let (status, _) = client.response().await.unwrap();
    assert!(status >= 400, "{status}");
    assert!(client.closed().await);
    // The body limit, well before the 10 s handler deadline.
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(90) && waited < Duration::from_secs(3),
        "{waited:?}"
    );
    // A complete body is answered and the connection kept.
    let mut client = Client::connect(address).await;
    client
        .send("POST /echo HTTP/1.1\r\nhost: localhost\r\ncontent-length: 3\r\n\r\nabc")
        .await;
    assert_eq!(client.response().await, Some((200, "abc".into())));
    client.send(&request_line("/fast")).await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
}

/// Stands for a client that reads its 60 s response slowly: the response is
/// written out in full although writing it takes longer than the idle
/// limit, because the idle gap starts only once it has been sent.
#[tokio::test]
async fn a_response_read_slowly_is_sent_in_full_within_the_response_limit() {
    // Idle 30 s and response 60 s scaled to 300 ms and 5 s; the client
    // waits 1 s before it reads at all.
    let limits = SHORT;
    // Small socket buffers, so most of the response waits in hyper.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_send_buffer_size(4096).unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = socket.listen(16).unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(serve_with(listener, router(), limits));
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(2048).unwrap();
    let mut client = Client(BufReader::with_capacity(
        64,
        socket.connect(address).await.unwrap(),
    ));
    client.send(&request_line("/large")).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (status, body) = client.response().await.unwrap();
    assert_eq!((status, body.len()), (200, LARGE));
}

/// The same through an in-memory pipe that holds 1 KiB, so it does not
/// depend on the kernel's socket buffer sizing.
#[tokio::test]
async fn a_response_read_slowly_through_a_small_pipe_is_sent_in_full() {
    let (near, far) = tokio::io::duplex(1024);
    tokio::spawn(serve::connection(far, router(), SHORT));
    let mut client = BufReader::new(near);
    client
        .get_mut()
        .write_all(request_line("/large").as_bytes())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut chunk = [0; 512];
        loop {
            match client.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => received.extend_from_slice(&chunk[..n]),
            }
            if received.ends_with(&[b'x'; 64]) && received.len() >= LARGE {
                let text = String::from_utf8_lossy(&received);
                if let Some((_, body)) = text.split_once("\r\n\r\n")
                    && body.len() >= LARGE
                {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    let text = String::from_utf8(received).unwrap();
    let (_, body) = text.split_once("\r\n\r\n").unwrap();
    assert_eq!(body.len(), LARGE);
}

/// A client that never reads its response is still cut off: at the
/// response limit, not held open by the unsent bytes.
#[tokio::test]
async fn a_response_never_read_is_cut_off_at_the_response_limit() {
    // Response 60 s scaled to 500 ms; idle 30 s to 5 s, so only the
    // response limit can end it this early.
    let limits = Limits {
        idle: Duration::from_secs(5),
        response: Duration::from_millis(500),
        ..SHORT
    };
    let (near, far) = tokio::io::duplex(1024);
    let served = tokio::spawn(serve::connection(far, router(), limits));
    let mut client = near;
    client
        .write_all(request_line("/large").as_bytes())
        .await
        .unwrap();
    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(10), served)
        .await
        .unwrap()
        .unwrap();
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(400) && waited < Duration::from_secs(3),
        "{waited:?}"
    );
    let mut received = Vec::new();
    client.read_to_end(&mut received).await.unwrap();
    assert!(received.len() < LARGE, "{}", received.len());
}

/// Far beyond the socket buffers, in one chunk, so hyper queues all of it
/// and is done with the body long before it can be sent.
const HUGE: usize = 1 << 20;

/// A response answered at once without reading the request body, whose
/// output a slow reader holds back. The body then arrives together with
/// the next request: hyper reads the body to its end with the answer
/// already in its write buffer, counts both as done, and starts the next
/// request while that answer is still unsent. That later request must not
/// extend the first response's limit: the connection is cut off at the
/// response limit counted from the first response's start.
#[tokio::test]
async fn a_later_request_does_not_extend_the_limit_of_an_unsent_response() {
    // Response 60 s scaled to 2 s; idle 30 s to 20 s, so only the response
    // limit can end the connection in time. The second request arrives at
    // 1.4 s: had it restarted the limit, the cut would come near 3.4 s.
    let limits = Limits {
        idle: Duration::from_secs(20),
        response: Duration::from_secs(2),
        ..SHORT
    };
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_send_buffer_size(4096).unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = socket.listen(16).unwrap();
    let address = listener.local_addr().unwrap();
    let client = tokio::net::TcpSocket::new_v4().unwrap();
    client.set_recv_buffer_size(2048).unwrap();
    let (client, accepted) = tokio::join!(client.connect(address), listener.accept());
    let mut client = client.unwrap();
    let served = tokio::spawn(serve::connection(accepted.unwrap().0, router(), limits));
    let started = Instant::now();
    // Only the headers: the answer is buffered before any of the body.
    client
        .write_all(b"POST /ignore HTTP/1.1\r\nhost: localhost\r\ncontent-length: 4\r\n\r\n")
        .await
        .unwrap();
    // The reader is slow: it reads nothing for now, so the answer stays
    // unsent. At 1.4 s the body arrives together with the next request.
    tokio::time::sleep(Duration::from_millis(1400)).await;
    client
        .write_all(format!("abcd{}", request_line("/fast")).as_bytes())
        .await
        .unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(10), served)
        .await
        .expect("the server ended the connection");
    ended.unwrap();
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(1800) && waited < Duration::from_millis(3000),
        "{waited:?}"
    );
    // Most of the answer was never sent.
    let mut received = 0;
    let mut chunk = [0; 4096];
    while let Ok(n @ 1..) = client.read(&mut chunk).await {
        received += n;
    }
    assert!(received < HUGE, "{received}");
}

#[tokio::test]
async fn connections_beyond_the_cap_are_closed_and_a_freed_place_is_reused() {
    let limits = Limits {
        idle: Duration::from_secs(30),
        ..Limits::default()
    };
    assert_eq!(limits.connections, 64);
    let address = start(router(), limits).await;
    let mut open = Vec::new();
    for _ in 0..limits.connections {
        let mut client = Client::connect(address).await;
        client.send(&request_line("/fast")).await;
        assert!(client.response().await.is_some());
        open.push(client);
    }
    let mut refused = Client::connect(address).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), refused.closed())
            .await
            .unwrap()
    );
    drop(open.pop());
    let served = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut client = Client::connect(address).await;
            client.send(&request_line("/fast")).await;
            if client.response().await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(served.is_ok(), "a freed place was not reused");
    // The others are still being served.
    open[0].send(&request_line("/fast")).await;
    assert_eq!(open[0].response().await, Some((200, "fast".into())));
}

/// Opens a stream and returns its status and the length of any error body,
/// keeping the connection open.
async fn open_stream(address: SocketAddr, session: char) -> (u16, usize, Client) {
    let mut client = Client::connect(address).await;
    client
        .send(&format!(
            "GET /api/v1/observations/stream HTTP/1.1\r\nhost: localhost\r\ncookie: __Host-limeos={}\r\n\r\n",
            session.to_string().repeat(64)
        ))
        .await;
    let mut line = String::new();
    client.0.read_line(&mut line).await.unwrap();
    let status = line.split(' ').nth(1).unwrap().parse().unwrap();
    let mut length = 0;
    loop {
        line.clear();
        client.0.read_line(&mut line).await.unwrap();
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap();
        }
    }
    (status, length, client)
}

#[tokio::test]
async fn one_session_holds_at_most_two_streams_and_others_are_unaffected() {
    let address = start(router_for(crate::read_tests::backend(1)), Limits::default()).await;
    let (first, _, _a1) = open_stream(address, 'a').await;
    let (second, _, a2) = open_stream(address, 'a').await;
    assert_eq!((first, second), (200, 200));
    let (third, length, mut refused) = open_stream(address, 'a').await;
    assert_eq!(third, 429);
    let mut body = vec![0; length];
    refused.0.read_exact(&mut body).await.unwrap();
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("live observation streams"), "{body}");
    for other in ['b', 'c', 'd'] {
        assert_eq!(open_stream(address, other).await.0, 200);
    }
    // Closing one of the session's streams frees its place.
    drop(a2);
    let reopened = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (status, _, client) = open_stream(address, 'a').await;
            if status == 200 {
                break client;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(reopened.is_ok());
}
fn router_for(backend: crate::read_tests::ReadBackend) -> Router {
    super::router(backend, "https://localhost".into())
}
