use bytes::Bytes;
use futures_util::Stream;

pub type ByteStream = std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>;

/// A response body is either fully buffered (small error payloads, non-streaming
/// JSON) or a live stream that must be relayed byte-for-byte.
pub enum ProxyBody {
    Bytes(Vec<u8>),
    Stream(ByteStream),
}

impl ProxyBody {
    pub fn bytes(text: impl Into<Vec<u8>>) -> Self {
        ProxyBody::Bytes(text.into())
    }
}
