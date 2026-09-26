// Independent TypeSpec peer authority for Scintilla WASI component admission.
// Generated JSON Schema is comparison evidence only; it is never the authored JSON Schema authority.
import "@typespec/json-schema";
using TypeSpec.JsonSchema;

namespace ScintillaWasiComponent;

@id("WasiRuntimeProfile")
enum WasiRuntimeProfile {
  wasi_0_2: "wasi_0_2",
  wasi_0_3: "wasi_0_3",
}

@id("InvocationKind")
enum InvocationKind {
  unary: "unary",
  server_stream: "server_stream",
  client_stream: "client_stream",
  bidi_stream: "bidi_stream",
}

@id("CapabilityKind")
enum CapabilityKind {
  outbound_network: "outbound_network",
  listen_network: "listen_network",
  filesystem_read: "filesystem_read",
  filesystem_write: "filesystem_write",
  clock: "clock",
  random: "random",
  env_config: "env_config",
  secret: "secret",
  kv: "kv",
  database: "database",
  queue: "queue",
  object_storage: "object_storage",
}

@id("CapabilityRequirement")
model CapabilityRequirement {
  kind: CapabilityKind;
  required: boolean;
  resource_id?: string;
}

@id("WasiComponentContractV1")
model WasiComponentContractV1 {
  schema_version: "scintilla.wasi-component/v1";
  component_id: string;
  wit_package: string;
  wit_world: string;
  runtime_profile: WasiRuntimeProfile;
  invocation_kind: InvocationKind;
  capabilities: CapabilityRequirement[];
}
