import 'dart:convert';
import 'dart:typed_data';

import '../lib/wire.dart';

final class CountingCodec implements StructuredWireCodec<Map<String, Object?>> {
  int decodeCalls = 0;

  @override
  StructuredCodecName get name => StructuredCodecName.json;

  @override
  String get contentType => name.contentType;

  @override
  Uint8List encode(Map<String, Object?> value) => Uint8List.fromList(utf8.encode(jsonEncode(value)));

  @override
  Map<String, Object?> decode(Uint8List bytes) {
    decodeCalls += 1;
    return (jsonDecode(utf8.decode(bytes, allowMalformed: false)) as Map).cast<String, Object?>();
  }
}

void expectThrows(void Function() callback) {
  var threw = false;
  try {
    callback();
  } catch (_) {
    threw = true;
  }
  assert(threw);
}

void main() {
  assert(sourcePrHead == 'c75075a7eb2086a17ef717310a24f72b288f5b99');
  assert(rawBinaryContentType == 'application/octet-stream');

  final body = Uint8List.fromList(utf8.encode('{"ok":true}'));
  final codec = CountingCodec();

  expectThrows(() {
    decodeStructuredResponse(
      ByteTransportResponse(status: 500, contentType: 'application/json', body: body),
      codec,
    );
  });
  assert(codec.decodeCalls == 0);

  expectThrows(() {
    decodeStructuredResponse(
      ByteTransportResponse(status: 200, contentType: 'application/msgpack', body: body),
      codec,
    );
  });
  assert(codec.decodeCalls == 0);

  expectThrows(() {
    decodeStructuredResponse(
      ByteTransportResponse(status: 200, contentType: 'application/json', body: body),
      codec,
      maxResponseBytes: 1,
    );
  });
  assert(codec.decodeCalls == 0);

  final decoded = decodeStructuredResponse(
    ByteTransportResponse(status: 200, contentType: 'Application/JSON; charset=utf-8', body: body),
    codec,
  );
  assert(decoded['ok'] == true);
  assert(codec.decodeCalls == 1);
}
