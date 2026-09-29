import 'dart:convert';
import 'dart:typed_data';

enum StructuredCodecName { json, messagePack, cbor, protobuf }

extension StructuredCodecMediaType on StructuredCodecName {
  String get contentType {
    switch (this) {
      case StructuredCodecName.json:
        return 'application/json';
      case StructuredCodecName.messagePack:
        return 'application/msgpack';
      case StructuredCodecName.cbor:
        return 'application/cbor';
      case StructuredCodecName.protobuf:
        return 'application/x-protobuf';
    }
  }
}

const rawBinaryContentType = 'application/octet-stream';
const defaultMaxBinaryResponseBytes = 8 * 1024 * 1024;
const sourcePrHead = 'c75075a7eb2086a17ef717310a24f72b288f5b99';

abstract interface class StructuredWireCodec<T> {
  StructuredCodecName get name;
  String get contentType;
  Uint8List encode(T value);
  T decode(Uint8List bytes);
}

final class JsonWireCodec<T> implements StructuredWireCodec<T> {
  const JsonWireCodec({required this.toJson, required this.fromJson});
  final Object? Function(T value) toJson;
  final T Function(Object? value) fromJson;
  @override
  StructuredCodecName get name => StructuredCodecName.json;
  @override
  String get contentType => name.contentType;
  @override
  Uint8List encode(T value) => Uint8List.fromList(utf8.encode(jsonEncode(toJson(value))));
  @override
  T decode(Uint8List bytes) => fromJson(jsonDecode(utf8.decode(bytes, allowMalformed: false)));
}

final class ByteTransportResponse {
  const ByteTransportResponse({required this.status, required this.contentType, required this.body});
  final int status;
  final String contentType;
  final Uint8List body;
}

final class ByteTransportResponseException implements Exception {
  const ByteTransportResponseException({required this.message, required this.status, required this.expectedContentType, required this.actualContentType});
  final String message;
  final int status;
  final String expectedContentType;
  final String actualContentType;
}

String normalizeMediaType(String value) => value.split(';').first.trim().toLowerCase();

T decodeStructuredResponse<T>(ByteTransportResponse response, StructuredWireCodec<T> codec, {int maxResponseBytes = defaultMaxBinaryResponseBytes}) {
  if (maxResponseBytes < 1) {
    throw RangeError.value(maxResponseBytes, 'maxResponseBytes', 'must be positive');
  }
  final expectedContentType = normalizeMediaType(codec.contentType);
  final actualContentType = normalizeMediaType(response.contentType);
  if (response.body.lengthInBytes > maxResponseBytes) {
    throw ByteTransportResponseException(message: 'too large', status: response.status, expectedContentType: expectedContentType, actualContentType: actualContentType);
  }
  if (response.status < 200 || response.status >= 300) {
    throw ByteTransportResponseException(message: 'bad status', status: response.status, expectedContentType: expectedContentType, actualContentType: actualContentType);
  }
  if (actualContentType != expectedContentType) {
    throw ByteTransportResponseException(message: 'wrong media', status: response.status, expectedContentType: expectedContentType, actualContentType: actualContentType);
  }
  return codec.decode(response.body);
}
