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
    net::{TcpListener, TcpStream},
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
    /// its headers.
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

async fn connection(stream: TcpStream, app: Router, limits: Limits) {
    let activity = Arc::new(Activity::new());
    let io = hyper_util::rt::TokioIo::new(Watched {
        stream,
        activity: activity.clone(),
    });
    let requests = activity.clone();
    let service = hyper::service::service_fn(move |request: hyper::Request<Incoming>| {
        let activity = requests.clone();
        activity.set(Phase::Busy);
        let request = request.map(|body| {
            Body::new(BodyDeadline {
                body,
                sleep: Box::pin(tokio::time::sleep(limits.body)),
            })
        });
        let app = app.clone();
        async move {
            let Ok(response) = app.oneshot(request).await;
            Ok::<_, Infallible>(response.map(|body| Body::new(Tracked { body, activity })))
        }
    });
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(hyper_util::rt::TokioTimer::new())
        // Hyper's own header timer also runs while idle between requests,
        // which would make the idle limit the header limit. `Activity`
        // times both instead.
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
/// Which limit applies to a connection now, and since when.
struct Activity {
    state: Mutex<(Phase, Instant)>,
    changed: Notify,
}
impl Activity {
    fn new() -> Self {
        Self {
            state: Mutex::new((Phase::Head, Instant::now())),
            changed: Notify::new(),
        }
    }
    fn set(&self, phase: Phase) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = (phase, Instant::now());
        self.changed.notify_one();
    }
    /// Bytes arrived: the first byte after an idle gap starts a request.
    fn read(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.0 == Phase::Idle {
            *state = (Phase::Head, Instant::now());
            drop(state);
            self.changed.notify_one();
        }
    }
    fn deadline(&self, limits: &Limits) -> Instant {
        let (phase, since) = *self.state.lock().unwrap_or_else(PoisonError::into_inner);
        since
            + match phase {
                Phase::Head => limits.header,
                Phase::Busy => limits.response,
                Phase::Idle => limits.idle,
            }
    }
}

/// The socket, noting when request bytes arrive.
struct Watched {
    stream: TcpStream,
    activity: Arc<Activity>,
}
impl AsyncRead for Watched {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.stream).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            this.activity.read();
        }
        result
    }
}
impl AsyncWrite for Watched {
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
        self.activity.set(Phase::Idle);
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
