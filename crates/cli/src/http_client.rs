use http::{HeaderMap, HeaderName, HeaderValue};

use crate::body::ProxyBody;
use crate::engine::{ProxyRequest, ProxyResponse, UpstreamClient};

/// The real transport. Bodies are handed back as a live stream so a long agent
/// response is relayed byte-for-byte rather than buffered.
pub struct ReqwestClient {
    client: reqwest::Client,
}

impl ReqwestClient {
    pub fn new() -> Self {
        ReqwestClient {
            client: reqwest::Client::builder()
                .build()
                .expect("reqwest client builds"),
        }
    }
}

impl Default for ReqwestClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UpstreamClient for ReqwestClient {
    async fn send(&self, request: ProxyRequest) -> Result<ProxyResponse, String> {
        let method =
            reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|e| e.to_string())?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in request.headers.iter() {
            builder = builder.header(name.as_str(), value.as_bytes());
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }

        let response = builder.send().await.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();

        let mut headers = HeaderMap::new();
        for (name, value) in response.headers().iter() {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_str().as_bytes()),
                HeaderValue::from_bytes(value.as_bytes()),
            ) {
                headers.insert(name, value);
            }
        }

        let stream = response.bytes_stream();
        let mapped = futures_util::StreamExt::map(stream, |chunk| {
            chunk.map_err(|e| e.to_string())
        });
        Ok(ProxyResponse {
            status,
            headers,
            body: ProxyBody::Stream(Box::pin(mapped)),
        })
    }
}
