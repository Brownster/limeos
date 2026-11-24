//! One HTTP request through the real router, over loopback, to a real core:
//! the API's body decoding and core's ordering of authentication, CSRF,
//! authorization and validation together, as a browser meets them.
use super::Core;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The status code of a session-authenticated JSON POST.
pub(crate) async fn post_status(
    core: &Core,
    path: &str,
    token: &str,
    csrf: &str,
    body: &str,
) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(limeos_api::serve(
        listener,
        limeos_api::router(core.clone(), "https://localhost".into()),
    ));
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nOrigin: https://localhost\r\nCookie: __Host-limeos={token}\r\nX-CSRF-Token: {csrf}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    server.abort();
    response
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status line: {response:?}"))
}
