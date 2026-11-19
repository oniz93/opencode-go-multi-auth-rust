use std::convert::Infallible;
use std::io;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio_stream::wrappers::ReceiverStream;

use crate::body::ProxyBody;
use crate::engine::{Engine, IncomingRequest, ProxyOutcome};
use crate::http_client::ReqwestClient;

type RespBody = BoxBody<Bytes, io::Error>;

/// The reverse proxy listener. One `Engine` guards the routing state; the lock
/// is held only while a decision is made, not for the life of a stream.
pub struct ProxyServer {
    engine: Arc<Engine>,
}

impl ProxyServer {
    pub fn new(engine: Engine) -> Self {
        ProxyServer {
            engine: Arc::new(engine),
        }
    }

    pub fn engine(&self) -> Arc<Engine> {
        Arc::clone(&self.engine)
    }

    /// Accept connections until the listener is dropped, serving each on its
    /// own task.
    pub async fn serve(self, listener: tokio::net::TcpListener) -> anyhow::Result<()> {
        loop {
            let (stream, _peer) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let engine = Arc::clone(&self.engine);
            let client = Arc::new(ReqwestClient::new());
            tokio::spawn(async move {
                let service = service_fn(move |req| {
                    let engine = Arc::clone(&engine);
                    let client = Arc::clone(&client);
                    async move { Ok::<_, Infallible>(handle(req, engine, client).await) }
                });
                if let Err(err) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    eprintln!("proxy connection error: {err}");
                }
            });
        }
    }
}

fn is_loopback_host(authority: &str) -> bool {
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

/// The proxy spends real credentials and has no auth of its own, so it only
/// serves local programs. Browsers always send `Origin` on cross-site requests,
/// and a DNS-rebinding page arrives with its own hostname in `Host`; both are
/// refused. Loopback origins (a local tool's own page) are allowed.
pub fn is_local_request(headers: &http::HeaderMap) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let host_ok = header("host").is_none_or(is_loopback_host);
    let origin_ok = header("origin").is_none_or(|origin| {
        origin
            .split_once("://")
            .is_some_and(|(_, authority)| is_loopback_host(authority))
    });
    host_ok && origin_ok
}

async fn handle(
    req: Request<Incoming>,
    engine: Arc<Engine>,
    client: Arc<ReqwestClient>,
) -> Response<RespBody> {
    if !is_local_request(req.headers()) {
        return into_response(ProxyOutcome {
            status: 403,
            headers: http::HeaderMap::new(),
            body: ProxyBody::bytes("{\"error\":\"ocrouter only serves local clients\"}"),
        });
    }
    let method = req.method().to_string();
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let headers = req.headers().clone();
    let body = req
        .into_body()
        .collect()
        .await
        .map(|b| b.to_bytes().to_vec())
        .unwrap_or_default();

    let incoming = IncomingRequest {
        method,
        path,
        headers,
        body,
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let outcome = engine.handle(incoming, client.as_ref(), now_ms).await;

    into_response(outcome)
}

fn into_response(outcome: ProxyOutcome) -> Response<RespBody> {
    let ProxyOutcome {
        status,
        headers,
        body,
    } = outcome;

    let mut builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if lower == "content-length" || lower == "transfer-encoding" || lower == "connection" {
            continue;
        }
        builder = builder.header(name, value);
    }

    // A channel decouples the upstream stream from the response body, which
    // keeps the body `Send + Sync` for hyper's per-connection task.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    pump(body, tx);

    let hyper_body = BodyExt::boxed(StreamBody::new(ReceiverStream::new(rx).map_ok(Frame::data)));

    builder
        .body(hyper_body)
        .unwrap_or_else(|_| Response::new(BoxBody::default()))
}

fn pump(body: ProxyBody, tx: tokio::sync::mpsc::Sender<Result<Bytes, io::Error>>) {
    match body {
        ProxyBody::Bytes(bytes) => {
            tokio::spawn(async move {
                let _ = tx.send(Ok(Bytes::from(bytes))).await;
            });
        }
        ProxyBody::Stream(mut stream) => {
            tokio::spawn(async move {
                while let Some(chunk) = stream.next().await {
                    if tx.send(chunk.map_err(io::Error::other)).await.is_err() {
                        break;
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_local_request;

    fn headers(pairs: &[(&str, &str)]) -> http::HeaderMap {
        let mut h = http::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn local_clients_are_served() {
        assert!(is_local_request(&headers(&[("host", "127.0.0.1:18905")])));
        assert!(is_local_request(&headers(&[("host", "localhost:18905")])));
        assert!(is_local_request(&headers(&[("host", "[::1]:18905")])));
        assert!(is_local_request(&headers(&[])));
        assert!(is_local_request(&headers(&[
            ("host", "127.0.0.1:18905"),
            ("origin", "http://localhost:3000"),
        ])));
    }

    #[test]
    fn browsers_and_rebound_hosts_are_refused() {
        assert!(!is_local_request(&headers(&[
            ("host", "127.0.0.1:18905"),
            ("origin", "https://evil.example"),
        ])));
        assert!(!is_local_request(&headers(&[
            ("host", "127.0.0.1:18905"),
            ("origin", "null"),
        ])));
        assert!(!is_local_request(&headers(&[(
            "host",
            "rebind.evil.example:18905"
        )])));
        assert!(!is_local_request(&headers(&[(
            "host",
            "127.0.0.1.evil.example"
        )])));
    }
}
