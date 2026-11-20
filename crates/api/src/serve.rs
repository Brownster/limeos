//! The HTTP/1 accept loop. Every time limit applies to one request or one
//! idle gap, never to the whole connection, so a keep-alive connection from
//! the proxy lives as long as it keeps being used and a request in flight is
//! never cut off by the connection's age.
use axum::{
    Router,
    body::{Body, Bytes},
};
use hyper::body::{Frame, Incoming, SizeHint};
use std::{
    convert::Infallible,
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpListener,
    sync::{Notify, Semaphore},
    time::{Instant, Sleep},
};
use tower::ServiceExt;

/// Connection and per-request bounds. The handler deadline (10 s, streams
/// excepted) is applied by the router's `limits` middleware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Open connections; a connection beyond it is closed at once.
    pub connections: usize,
    /// From a connection's start, or a request's first byte, to the end of
    /// its headers. A request whose first bytes arrived while the one before
    /// it was still being answered (pipelined, or in the same packet) is
    /// timed from the end of that answer.
    pub header: Duration,
    /// From the end of the headers to the end of the request body.
    pub body: Duration,
    /// From the end of one response to the next request's first byte.
    pub idle: Duration,
    /// From the end of the headers to the end of the response, so a client
    /// that stops reading cannot hold a connection; above the 25 s stream
    /// lifetime plus the handler deadline.
    pub response: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            connections: 64,
            header: Duration::from_secs(5),
            body: Duration::from_secs(5),
            idle: Duration::from_secs(30),
            response: Duration::from_secs(60),
        }
    }
}

pub async fn serve(listener: TcpListener, router: Router) -> io::Result<()> {
    serve_with(listener, router, Limits::default()).await
}

pub async fn serve_with(listener: TcpListener, router: Router, limits: Limits) -> io::Result<()> {
    let permits = Arc::new(Semaphore::new(limits.connections));
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            // Descriptor exhaustion or an aborted peer: the listener still
            // works, so pause and keep serving instead of ending core.
            Err(error) if limeos_contracts::accept_retryable(&error) => {
                tokio::time::sleep(limeos_contracts::ACCEPT_PAUSE).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let app = router.clone();
        tokio::spawn(async move {
            let _permit = permit;
            connection(stream, app, limits).await;
        });
    }
}

pub(crate) async fn connection<I>(stream: I, app: Router, limits: Limits)
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let activity = Arc::new(Activity::new());
    let io = hyper_util::rt::TokioIo::new(Watched {
        stream,
        activity: activity.clone(),
    });
    let requests = activity.clone();
    let service = hyper::service::service_fn(move |request: hyper::Request<Incoming>| {
        let activity = requests.clone();
        // Hyper's own view of the body's length on the wire; `None` for a
        // chunked body, whose end this connection does not track.
        let tracked = activity.request(hyper::body::Body::size_hint(request.body()).exact());
        let request = request.map(|body| {
            Body::new(BodyDeadline {
                body,
                sleep: Box::pin(tokio::time::sleep(limits.body)),
            })
        });
        let app = app.clone();
        async move {
            let Ok(mut response) = app.oneshot(request).await;
            if !tracked {
                // Where the next request starts is unknown, so its header
                // limit could not be applied: answer and close instead.
                response.headers_mut().insert(
                    axum::http::header::CONNECTION,
                    axum::http::HeaderValue::from_static("close"),
                );
            }
            Ok::<_, Infallible>(response.map(|body| Body::new(Tracked { body, activity })))
        }
    });
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(hyper_util::rt::TokioTimer::new())
        // Hyper's own header timer also runs while idle between requests,
        // which would make the idle limit the header limit. `Activity`
        // times both instead, from the request boundaries it tracks.
        .header_read_timeout(None)
        .max_headers(32)
        .max_buf_size(16 * 1024)
        .keep_alive(true);
    let connection = builder.serve_connection(io, service);
    tokio::pin!(connection);
    loop {
        let deadline = activity.deadline(&limits);
        tokio::select! {
            _ = connection.as_mut() => return,
            () = activity.changed.notified() => {}
            () = tokio::time::sleep_until(deadline) => {
                if Instant::now() >= activity.deadline(&limits) {
                    return;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Reading a request's headers.
    Head,
    /// Headers read; the request is being handled or its response written.
    Busy,
    /// Between a response and the next request.
    Idle,
}
/// Where the bytes handed to hyper stand against request boundaries. Hyper
/// reads ahead: one socket read can carry the end of one request and the
/// start of the next, which hyper keeps buffered while it serves the first.
/// Following the boundaries, not the reads, lets the header limit apply to
/// a request whose first bytes were read early.
enum Stream {
    /// In a request's headers.
    Head(HeadScan),
    /// The headers ended; hyper has not yet said how long the body is.
    /// `after` holds the bytes handed over after the headers meanwhile.
    Parsed { after: Vec<u8> },
    /// In a body with `remaining` bytes left on the wire.
    Body { remaining: u64 },
    /// A body whose end is not tracked (chunked); the response closes the
    /// connection, so no later request is read.
    Untracked,
}
/// Finds the blank line that ends a request's headers, as hyper's parser
/// does: lines end with LF or CRLF, and blank lines before the request
/// line are skipped.
#[derive(Clone, Copy, Default)]
struct HeadScan {
    /// A byte other than CR or LF has been seen in this request.
    started: bool,
    /// The current line has such a byte.
    line: bool,
}
impl HeadScan {
    /// The index just past the headers' end in `bytes`, if it is there.
    fn feed(&mut self, bytes: &[u8]) -> Option<usize> {
        for (index, byte) in bytes.iter().enumerate() {
            match byte {
                b'\n' if self.started && !self.line => return Some(index + 1),
                b'\n' => self.line = false,
                b'\r' => {}
                _ => {
                    self.started = true;
                    self.line = true;
                }
            }
        }
        None
    }
}
/// More than hyper buffers (`max_buf_size`); beyond it the boundaries are
/// given up and the connection closes after the current response.
const MAX_AFTER_HEAD: usize = 64 * 1024;
struct State {
    phase: Phase,
    since: Instant,
    stream: Stream,
    /// A later request's first bytes reached hyper while this one was busy.
    next: bool,
}
/// Which limit applies to a connection now, and since when.
struct Activity {
    state: Mutex<State>,
    changed: Notify,
}
impl State {
    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.since = Instant::now();
    }
    /// Accounts for bytes handed to hyper.
    fn delivered(&mut self, mut bytes: &[u8]) {
        loop {
            match &mut self.stream {
                Stream::Head(scan) => {
                    let end = scan.feed(bytes);
                    if scan.started {
                        match self.phase {
                            // The first byte after an idle gap starts a request.
                            Phase::Idle => self.enter(Phase::Head),
                            Phase::Busy => self.next = true,
                            Phase::Head => {}
                        }
                    }
                    if let Some(end) = end {
                        self.stream = Stream::Parsed {
                            after: bytes[end..].to_vec(),
                        };
                    }
                    return;
                }
                Stream::Parsed { after } => {
                    if after.len() + bytes.len() > MAX_AFTER_HEAD {
                        self.stream = Stream::Untracked;
                    } else {
                        after.extend_from_slice(bytes);
                    }
                    return;
                }
                Stream::Body { remaining } => {
                    let taken = bytes
                        .len()
                        .min(usize::try_from(*remaining).unwrap_or(usize::MAX));
                    *remaining -= taken as u64;
                    bytes = &bytes[taken..];
                    if *remaining > 0 {
                        return;
                    }
                    self.stream = Stream::Head(HeadScan::default());
                    if bytes.is_empty() {
                        return;
                    }
                }
                Stream::Untracked => return,
            }
        }
    }
}
impl Activity {
    fn new() -> Self {
        Self {
            state: Mutex::new(State {
                phase: Phase::Head,
                since: Instant::now(),
                stream: Stream::Head(HeadScan::default()),
                next: false,
            }),
            changed: Notify::new(),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Hyper parsed a request's headers; `body` is its length on the wire.
    /// Returns whether the request's end is tracked.
    fn request(&self, body: Option<u64>) -> bool {
        let mut state = self.lock();
        state.enter(Phase::Busy);
        state.next = false;
        let after = match std::mem::replace(&mut state.stream, Stream::Untracked) {
            Stream::Parsed { after } => Some(after),
            _ => None,
        };
        let tracked = match (after, body) {
            (Some(after), Some(remaining)) => {
                state.stream = Stream::Body { remaining };
                state.delivered(&after);
                true
            }
            _ => false,
        };
        drop(state);
        self.changed.notify_one();
        tracked
    }
    /// The response is done: the next request's limit starts. Bytes of it
    /// already with hyper start its header limit now.
    fn finished(&self) {
        let mut state = self.lock();
        let next = std::mem::take(&mut state.next);
        state.enter(if next { Phase::Head } else { Phase::Idle });
        drop(state);
        self.changed.notify_one();
    }
    /// Bytes arrived from the socket and were handed to hyper.
    fn read(&self, bytes: &[u8]) {
        let mut state = self.lock();
        let before = (state.phase, state.since);
        state.delivered(bytes);
        let changed = (state.phase, state.since) != before;
        drop(state);
        if changed {
            self.changed.notify_one();
        }
    }
    fn deadline(&self, limits: &Limits) -> Instant {
        let state = self.lock();
        state.since
            + match state.phase {
                Phase::Head => limits.header,
                Phase::Busy => limits.response,
                Phase::Idle => limits.idle,
            }
    }
}

/// The socket, noting which request the bytes read belong to.
struct Watched<I> {
    stream: I,
    activity: Arc<Activity>,
}
impl<I: AsyncRead + Unpin> AsyncRead for Watched<I> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.stream).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            this.activity.read(&buf.filled()[before..]);
        }
        result
    }
}
impl<I: AsyncWrite + Unpin> AsyncWrite for Watched<I> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write_vectored(cx, bufs)
    }
    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

/// A request body that fails once its deadline passes; the handler then
/// answers with an error and the connection closes with the body unread.
struct BodyDeadline {
    body: Incoming,
    sleep: Pin<Box<Sleep>>,
}
impl hyper::body::Body for BodyDeadline {
    type Data = Bytes;
    type Error = axum::BoxError;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.body).poll_frame(cx) {
            Poll::Ready(frame) => Poll::Ready(frame.map(|frame| frame.map_err(Into::into))),
            Poll::Pending => match this.sleep.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Some(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "request body deadline",
                )
                .into()))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
    fn is_end_stream(&self) -> bool {
        hyper::body::Body::is_end_stream(&self.body)
    }
    fn size_hint(&self) -> SizeHint {
        hyper::body::Body::size_hint(&self.body)
    }
}

/// A response body that starts the idle gap once hyper has finished with it.
struct Tracked {
    body: Body,
    activity: Arc<Activity>,
}
impl Drop for Tracked {
    fn drop(&mut self) {
        self.activity.finished();
    }
}
impl hyper::body::Body for Tracked {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        Pin::new(&mut self.get_mut().body).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool {
        hyper::body::Body::is_end_stream(&self.body)
    }
    fn size_hint(&self) -> SizeHint {
        hyper::body::Body::size_hint(&self.body)
    }
}
