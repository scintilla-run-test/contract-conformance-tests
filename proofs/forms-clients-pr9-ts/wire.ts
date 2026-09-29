export type StructuredCodecName = 'json' | 'messagepack' | 'cbor' | 'protobuf';

export const structuredCodecContentType = (codec: StructuredCodecName): string => {
  switch (codec) {
    case 'json': return 'application/json';
    case 'messagepack': return 'application/msgpack';
    case 'cbor': return 'application/cbor';
    case 'protobuf': return 'application/x-protobuf';
  }
};

export const RAW_BINARY_CONTENT_TYPE = 'application/octet-stream' as const;
export const DEFAULT_MAX_BINARY_RESPONSE_BYTES = 8 * 1024 * 1024;

export interface StructuredWireCodec<T> {
  readonly name: StructuredCodecName;
  readonly contentType: string;
  encode(value: T): Uint8Array;
  decode(bytes: Uint8Array): T;
}

export interface ByteTransportRequest {
  readonly method: 'GET' | 'POST';
  readonly path: string;
  readonly body?: Uint8Array;
  readonly accept: string;
  readonly contentType?: string;
}

export interface ByteTransportResponse {
  readonly status: number;
  readonly contentType: string;
  readonly body: Uint8Array;
}

export class ByteTransportResponseError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly expectedContentType: string,
    readonly actualContentType: string,
  ) {
    super(message);
    this.name = 'ByteTransportResponseError';
  }
}

export const normalizeMediaType = (value: string): string => {
  const [mediaType = ''] = value.split(';', 1);
  return mediaType.trim().toLowerCase();
};

export const decodeStructuredResponse = <T>(
  response: ByteTransportResponse,
  codec: StructuredWireCodec<T>,
  maxResponseBytes: number = DEFAULT_MAX_BINARY_RESPONSE_BYTES,
): T => {
  if (!Number.isSafeInteger(maxResponseBytes) || maxResponseBytes < 1) {
    throw new RangeError('maxResponseBytes must be a positive safe integer');
  }
  const expectedContentType = normalizeMediaType(codec.contentType);
  const actualContentType = normalizeMediaType(response.contentType);
  if (response.body.byteLength > maxResponseBytes) {
    throw new ByteTransportResponseError('too large', response.status, expectedContentType, actualContentType);
  }
  if (response.status < 200 || response.status >= 300) {
    throw new ByteTransportResponseError('bad status', response.status, expectedContentType, actualContentType);
  }
  if (actualContentType !== expectedContentType) {
    throw new ByteTransportResponseError('wrong media', response.status, expectedContentType, actualContentType);
  }
  return codec.decode(response.body);
};

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

export const jsonWireCodec = <T>(): StructuredWireCodec<T> => ({
  name: 'json',
  contentType: structuredCodecContentType('json'),
  encode(value: T): Uint8Array { return encoder.encode(JSON.stringify(value)); },
  decode(bytes: Uint8Array): T { return JSON.parse(decoder.decode(bytes)) as T; },
});

export const SOURCE_PR_HEAD = 'c75075a7eb2086a17ef717310a24f72b288f5b99';
