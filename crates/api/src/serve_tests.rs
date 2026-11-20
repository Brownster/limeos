//! `serve` over a real socket, with the limits shortened so the tests are
//! fast; each test says which default it stands for.
use super::*;
use axum::{body::Bytes, extract::Path};
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
        .layer(middleware::from_fn(limits))
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
            }
        }
        let mut body = vec![0; length];
        self.0.read_exact(&mut body).await.ok()?;
        Some((status, String::from_utf8(body).ok()?))
    }
    async fn closed(&mut self) -> bool {
        let mut byte = [0];
        matches!(self.0.read(&mut byte).await, Ok(0) | Err(_))
    }
}

/// Stands for a keep-alive connection from the proxy carrying a request at
/// the old 30 s connection age: the connection outlives the sum of every
/// limit, with a request in flight across that point, and is still served.
#[tokio::test]
async fn a_keep_alive_connection_older_than_every_limit_still_serves_a_request_in_flight() {
    let address = start(router(), SHORT).await;
    let started = Instant::now();
    let mut client = Client::connect(address).await;
    client.send(&request_line("/fast")).await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
    tokio::time::sleep(Duration::from_millis(200)).await;
    // In flight from about 200 ms to 900 ms; header + body + idle is 500 ms.
    client.send(&request_line("/sleep/700")).await;
    assert_eq!(client.response().await, Some((200, "slept".into())));
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        client.send(&request_line("/fast")).await;
        assert_eq!(client.response().await, Some((200, "fast".into())));
    }
    assert!(started.elapsed() >= Duration::from_millis(1500));
}

#[tokio::test]
async fn an_idle_keep_alive_connection_stays_open_for_the_idle_limit_and_then_closes() {
    let address = start(router(), SHORT).await;
    let mut client = Client::connect(address).await;
    client.send(&request_line("/fast")).await;
    assert!(client.response().await.is_some());
    // Longer than the header limit: idle time is not header time.
    tokio::time::sleep(Duration::from_millis(200)).await;
    client.send(&request_line("/fast")).await;
    assert_eq!(client.response().await, Some((200, "fast".into())));
    let idle = Instant::now();
    assert!(client.closed().await);
    let waited = idle.elapsed();
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_secs(3),
        "{waited:?}"
    );
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
                if let Some((_, body)) = text.split_once("\r\n\r\n") {
                    if body.len() >= LARGE {
                        break;
                    }
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
