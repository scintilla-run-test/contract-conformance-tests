import assert from 'node:assert/strict';
import {
  decodeStructuredResponse,
  jsonWireCodec,
  RAW_BINARY_CONTENT_TYPE,
  SOURCE_PR_HEAD,
  type StructuredWireCodec,
} from './wire.ts';

assert.equal(SOURCE_PR_HEAD, 'c75075a7eb2086a17ef717310a24f72b288f5b99');
assert.equal(RAW_BINARY_CONTENT_TYPE, 'application/octet-stream');

let decodeCalls = 0;
const countingCodec: StructuredWireCodec<{ ok: boolean }> = {
  ...jsonWireCodec<{ ok: boolean }>(),
  decode(bytes: Uint8Array) {
    decodeCalls += 1;
    return jsonWireCodec<{ ok: boolean }>().decode(bytes);
  },
};

const body = new TextEncoder().encode('{"ok":true}');

for (const response of [
  { status: 500, contentType: 'application/json', body },
  { status: 200, contentType: 'application/msgpack', body },
]) {
  assert.throws(() => decodeStructuredResponse(response, countingCodec));
  assert.equal(decodeCalls, 0);
}

assert.throws(() =>
  decodeStructuredResponse(
    { status: 200, contentType: 'application/json', body },
    countingCodec,
    1,
  ),
);
assert.equal(decodeCalls, 0);

const decoded = decodeStructuredResponse(
  { status: 200, contentType: 'Application/JSON; charset=utf-8', body },
  countingCodec,
);
assert.deepEqual(decoded, { ok: true });
assert.equal(decodeCalls, 1);
