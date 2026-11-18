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

async fn handle(
    req: Request<Incoming>,
    engine: Arc<Engine>,
    client: Arc<ReqwestClient>,
) -> Response<RespBody> {
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

    let hyper_body = BodyExt::boxed(StreamBody::new(
        ReceiverStream::new(rx).map_ok(Frame::data),
    ));

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
