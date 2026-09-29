use serde::{Serialize, de::DeserializeOwned};

use crate::Method;

pub const DEFAULT_MAX_BINARY_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub const RAW_BINARY_CONTENT_TYPE: &str = "application/octet-stream";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredCodec {
    Json,
    MessagePack,
    Cbor,
    Protobuf,
}

impl StructuredCodec {
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        return match self {
            Self::Json => "application/json",
            Self::MessagePack => "application/msgpack",
            Self::Cbor => "application/cbor",
            Self::Protobuf => "application/x-protobuf",
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteRequestSpec {
    pub method: Method,
    pub path: String,
    pub body: Option<Vec<u8>>,
    pub accept: &'static str,
    pub content_type: Option<&'static str>,
}

impl ByteRequestSpec {
    #[must_use]
    pub fn get(path: String, response: StructuredCodec) -> Self {
        return Self {
            method: Method::Get,
            path,
            body: None,
            accept: response.content_type(),
            content_type: None,
        };
    }

    #[must_use]
    pub fn post(
        path: String,
        body: Vec<u8>,
        request: StructuredCodec,
        response: StructuredCodec,
    ) -> Self {
        return Self {
            method: Method::Post,
            path,
            body: Some(body),
            accept: response.content_type(),
            content_type: Some(request.content_type()),
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireCodecError(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ByteResponseError {
    ResponseTooLarge { actual: usize, max: usize },
    HttpStatus { status: u16 },
    ContentTypeMismatch { expected: &'static str, actual: String },
    Decode(WireCodecError),
}

pub trait StructuredWireCodec<T> {
    fn codec(&self) -> StructuredCodec;
    fn encode(&self, value: &T) -> Result<Vec<u8>, WireCodecError>;
    fn decode(&self, bytes: &[u8]) -> Result<T, WireCodecError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct JsonWireCodec;

impl<T> StructuredWireCodec<T> for JsonWireCodec
where
    T: Serialize + DeserializeOwned,
{
    fn codec(&self) -> StructuredCodec {
        return StructuredCodec::Json;
    }

    fn encode(&self, value: &T) -> Result<Vec<u8>, WireCodecError> {
        return serde_json::to_vec(value).map_err(|error| WireCodecError(error.to_string()));
    }

    fn decode(&self, bytes: &[u8]) -> Result<T, WireCodecError> {
        return serde_json::from_slice(bytes).map_err(|error| WireCodecError(error.to_string()));
    }
}

pub fn decode_structured_response<T, C>(
    response: &ByteResponse,
    codec: &C,
) -> Result<T, ByteResponseError>
where
    C: StructuredWireCodec<T>,
{
    return decode_structured_response_with_limit(response, codec, DEFAULT_MAX_BINARY_RESPONSE_BYTES);
}

pub fn decode_structured_response_with_limit<T, C>(
    response: &ByteResponse,
    codec: &C,
    max_response_bytes: usize,
) -> Result<T, ByteResponseError>
where
    C: StructuredWireCodec<T>,
{
    if response.body.len() > max_response_bytes {
        return Err(ByteResponseError::ResponseTooLarge {
            actual: response.body.len(),
            max: max_response_bytes,
        });
    }
    if !(200..300).contains(&response.status) {
        return Err(ByteResponseError::HttpStatus { status: response.status });
    }
    let expected = codec.codec().content_type();
    let actual = normalize_media_type(&response.content_type);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(ByteResponseError::ContentTypeMismatch { expected, actual });
    }
    return codec.decode(&response.body).map_err(ByteResponseError::Decode);
}

fn normalize_media_type(value: &str) -> String {
    return value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingCodec(std::cell::Cell<usize>);

    impl StructuredWireCodec<serde_json::Value> for CountingCodec {
        fn codec(&self) -> StructuredCodec {
            return StructuredCodec::Json;
        }
        fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, WireCodecError> {
            return serde_json::to_vec(value).map_err(|error| WireCodecError(error.to_string()));
        }
        fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, WireCodecError> {
            self.0.set(self.0.get() + 1);
            return serde_json::from_slice(bytes).map_err(|error| WireCodecError(error.to_string()));
        }
    }

    fn body() -> Vec<u8> {
        return br#"{"ok":true}"#.to_vec();
    }

    #[test]
    fn structured_builder_has_no_raw_representation_parameter() {
        let request = ByteRequestSpec::post(
            "/v1/forms".into(),
            body(),
            StructuredCodec::Json,
            StructuredCodec::Json,
        );
        assert_eq!(request.content_type, Some("application/json"));
        assert_eq!(request.accept, "application/json");
        assert_eq!(RAW_BINARY_CONTENT_TYPE, "application/octet-stream");
    }

    #[test]
    fn rejects_before_decoder() {
        for response in [
            ByteResponse { status: 500, content_type: "application/json".into(), body: body() },
            ByteResponse { status: 200, content_type: "application/msgpack".into(), body: body() },
        ] {
            let codec = CountingCodec(std::cell::Cell::new(0));
            assert!(decode_structured_response::<serde_json::Value, _>(&response, &codec).is_err());
            assert_eq!(codec.0.get(), 0);
        }
        let codec = CountingCodec(std::cell::Cell::new(0));
        let response = ByteResponse { status: 200, content_type: "application/json".into(), body: body() };
        assert!(decode_structured_response_with_limit::<serde_json::Value, _>(&response, &codec, 1).is_err());
        assert_eq!(codec.0.get(), 0);
    }
}
