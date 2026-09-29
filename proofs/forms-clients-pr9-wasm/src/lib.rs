#![forbid(unsafe_code)]

pub const SOURCE_PR_HEAD: &str = "c75075a7eb2086a17ef717310a24f72b288f5b99";
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

#[must_use]
pub fn structured_codec_content_type(codec: &str) -> Option<&'static str> {
    return match codec {
        "json" => Some(StructuredCodec::Json.content_type()),
        "messagepack" => Some(StructuredCodec::MessagePack.content_type()),
        "cbor" => Some(StructuredCodec::Cbor.content_type()),
        "protobuf" => Some(StructuredCodec::Protobuf.content_type()),
        _ => None,
    };
}

/// Raw remains media metadata. There is deliberately no generic raw route or
/// request constructor in this WASM projection.
#[must_use]
pub const fn raw_binary_content_type() -> &'static str {
    return RAW_BINARY_CONTENT_TYPE;
}

/// WASM callers preserve bytes as bytes; no UTF-8/base64 coercion is applied.
#[must_use]
pub fn copy_binary(input: &[u8]) -> Vec<u8> {
    return input.to_vec();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_registry_matches_contract() {
        assert_eq!(structured_codec_content_type("json"), Some("application/json"));
        assert_eq!(
            structured_codec_content_type("messagepack"),
            Some("application/msgpack")
        );
        assert_eq!(structured_codec_content_type("cbor"), Some("application/cbor"));
        assert_eq!(
            structured_codec_content_type("protobuf"),
            Some("application/x-protobuf")
        );
        assert_eq!(structured_codec_content_type("raw"), None);
    }

    #[test]
    fn raw_is_metadata_only_and_bytes_round_trip_exactly() {
        assert_eq!(raw_binary_content_type(), "application/octet-stream");
        let input = [0_u8, 1, 2, 127, 128, 254, 255];
        assert_eq!(copy_binary(&input), input);
    }
}
