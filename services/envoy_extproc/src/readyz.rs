//! SLICE 6 — `/readyz` + `/livez` HTTP probe.
//!
//! Per design §3.3 carve-out, the readiness probe is deferred from
//! SLICE 1 (where it would have required Kubernetes wiring before the
//! routing-extraction work). SLICE 6 lands it alongside the Helm
//! sub-chart so the production Deployment's `readinessProbe.httpGet`
//! and `livenessProbe.httpGet` have a target.
//!
//! Contract:
//!   * `GET /livez` always returns 200 OK with `ok` body — process is
//!     alive (Kubernetes only restarts on this if the listener itself
//!     dies, which collapses the gRPC server too).
//!   * `GET /readyz` returns 200 OK when the supplied `ready` atomic is
//!     `true`, 503 Service Unavailable otherwise. The main() boot
//!     sequence flips it to `true` only after both:
//!       - the sidecar startup handshake succeeded, AND
//!       - the SidecarClient (mTLS-TCP channel or UDS dev channel)
//!         was successfully constructed.
//!     A SIGTERM flips it back to `false` before draining.
//!   * Any other path returns 404 Not Found with a tiny body. We do NOT
//!     surface internal metrics here — the dedicated metrics endpoint
//!     comes in SLICE 7 / future hardening.
//!
//! Implementation: hyper v1 server bound on `cfg.readyz_addr`
//! (`SPENDGUARD_EXTPROC_READYZ_ADDR`, default `0.0.0.0:9090`). The
//! background task is owned by main(); SIGTERM aborts the JoinHandle.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.3 (carve-out)
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §7.1 (readiness probe Blocker)

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// SLICE 6 — bind the `/readyz` + `/livez` listener and spawn the
/// accept loop on a tokio task. Returns the `JoinHandle` so main() can
/// abort the task on shutdown (the underlying TcpListener drop closes
/// the socket).
///
/// Fail-fast: if the address is already in use we return the bind
/// error so main() exits non-zero rather than silently leaving
/// Kubernetes without a readiness signal.
pub async fn spawn(addr: SocketAddr, ready: Arc<AtomicBool>) -> anyhow::Result<JoinHandle<()>> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind /readyz listener at {addr}: {e}"))?;
    let actual = listener.local_addr().unwrap_or(addr);
    info!(addr = %actual, "SLICE 6 readyz/livez listener bound");

    let handle = tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    warn!(err = %e, "readyz listener accept failed; continuing");
                    continue;
                }
            };
            let ready = ready.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req| {
                    let ready = ready.clone();
                    async move { handle_request(req, ready).await }
                });
                if let Err(e) = http1::Builder::new()
                    .keep_alive(false)
                    .serve_connection(io, svc)
                    .await
                {
                    debug!(peer = %peer, err = %e, "readyz connection error");
                }
            });
        }
    });
    Ok(handle)
}

/// Route the request to the appropriate handler. The shape is
/// deliberately tiny — the probe is on the critical path of every K8s
/// scrape so we keep allocations to the bare minimum (Bytes::from of
/// a static slice does not allocate).
async fn handle_request(
    req: Request<hyper::body::Incoming>,
    ready: Arc<AtomicBool>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path();
    let response = match path {
        "/livez" => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/plain; charset=utf-8")
            .body(Full::new(Bytes::from_static(b"ok\n")))
            .expect("livez response build"),
        "/readyz" => {
            if ready.load(Ordering::SeqCst) {
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body(Full::new(Bytes::from_static(b"ready\n")))
                    .expect("readyz ok response build")
            } else {
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body(Full::new(Bytes::from_static(b"not ready\n")))
                    .expect("readyz 503 response build")
            }
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "text/plain; charset=utf-8")
            .body(Full::new(Bytes::from_static(b"not found\n")))
            .expect("404 response build"),
    };
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    /// Helper: bind a probe listener on a random port, return the
    /// `JoinHandle` + the bound port. The probe is owned by the task so
    /// the caller only needs the port to issue raw HTTP requests.
    async fn spawn_test_listener(ready: Arc<AtomicBool>) -> (JoinHandle<()>, u16) {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        // We bind a listener here just to learn the port, drop it, then
        // re-bind via spawn(). This is racy but the SLICE 1 smoke tests
        // already use the same pattern and have not flaked.
        let probe_listener = TcpListener::bind(addr).await.unwrap();
        let port = probe_listener.local_addr().unwrap().port();
        drop(probe_listener);
        let real_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let handle = spawn(real_addr, ready).await.expect("readyz spawn");
        // Give the accept loop one tick to wire up.
        tokio::time::sleep(Duration::from_millis(20)).await;
        (handle, port)
    }

    /// SLICE 6 — `/livez` always returns 200, regardless of the ready
    /// state.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn livez_always_returns_200() {
        let ready = Arc::new(AtomicBool::new(false));
        let (handle, port) = spawn_test_listener(ready).await;
        let body = http_get(&format!("127.0.0.1:{port}"), "/livez").await;
        handle.abort();
        assert!(body.starts_with("HTTP/1.1 200"), "livez body: {body}");
        assert!(body.ends_with("ok\n"), "livez body: {body}");
    }

    /// SLICE 6 — `/readyz` returns 503 before main() flips the flag.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn readyz_returns_503_when_not_ready() {
        let ready = Arc::new(AtomicBool::new(false));
        let (handle, port) = spawn_test_listener(ready).await;
        let body = http_get(&format!("127.0.0.1:{port}"), "/readyz").await;
        handle.abort();
        assert!(body.starts_with("HTTP/1.1 503"), "readyz body: {body}");
    }

    /// SLICE 6 — `/readyz` returns 200 after main() flips the flag.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn readyz_returns_200_when_ready() {
        let ready = Arc::new(AtomicBool::new(false));
        let (handle, port) = spawn_test_listener(ready.clone()).await;
        ready.store(true, Ordering::SeqCst);
        let body = http_get(&format!("127.0.0.1:{port}"), "/readyz").await;
        handle.abort();
        assert!(body.starts_with("HTTP/1.1 200"), "readyz body: {body}");
        assert!(body.ends_with("ready\n"), "readyz body: {body}");
    }

    /// SLICE 6 — unknown paths return 404. Defense in depth so a
    /// probe scraper that types `/metrics` does not get a misleading
    /// 200 from a route we didn't actually mean to expose.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unknown_path_returns_404() {
        let ready = Arc::new(AtomicBool::new(true));
        let (handle, port) = spawn_test_listener(ready).await;
        let body = http_get(&format!("127.0.0.1:{port}"), "/metrics").await;
        handle.abort();
        assert!(body.starts_with("HTTP/1.1 404"), "404 body: {body}");
    }

    /// Tiny HTTP/1.1 GET — keeps test deps minimal. Returns the full
    /// raw response (headers + body).
    async fn http_get(host: &str, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    }
}
