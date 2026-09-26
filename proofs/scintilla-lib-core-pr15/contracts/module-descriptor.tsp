import "@typespec/json-schema";

using TypeSpec.JsonSchema;

namespace Scintilla.ModuleContracts;

enum ModuleKind {
  lambda,
  http,
  rpc,
  worker,
  actor,
  container,
}

enum InvocationMode {
  request_response,
  stream,
  event,
  long_lived,
}

enum ConcurrencyModel {
  concurrent,
  serial,
  keyed_serial,
}

enum CancellationModel {
  deadline,
  cooperative,
  process_termination,
}

enum IsolationRequirement {
  runtime_default,
  process,
  container,
  micro_vm,
}

enum Capability {
  clock,
  random,
  network_outbound,
  network_listen,
  filesystem_read,
  filesystem_write,
  process_spawn,
  env_read,
  secrets_read,
  database_read,
  database_write,
  key_value_read,
  key_value_write,
  queue_publish,
  queue_consume,
  object_storage_read,
  object_storage_write,
}

model ExecutionProfile {
  invocation: InvocationMode;
  concurrency: ConcurrencyModel;
  cancellation: CancellationModel;
  isolation: IsolationRequirement;

  @minValue(0)
  default_timeout_ms: uint64;

  @minValue(1)
  max_concurrency?: uint32 | null;

  idempotent: boolean;
  retry_safe: boolean;
}

model ModuleDescriptorBase {
  contract_version: "scintilla-module-contract-v1";
  semantics_version?: "scintilla.module-semantics/v2" | null;
  context_abi?: "scintilla.context/v1" | null;

  @minLength(1)
  @pattern("^[A-Za-z0-9_.-]+$")
  name: string;

  @minLength(1)
  @pattern("^\\S+$")
  entrypoint: string;

  execution?: ExecutionProfile;

  @uniqueItems
  capabilities?: Capability[];
}

model LambdaModuleDescriptor extends ModuleDescriptorBase { kind: "lambda"; `interface`: "scintilla.lambda.v1"; }
model HttpModuleDescriptor extends ModuleDescriptorBase { kind: "http"; `interface`: "scintilla.http.v1"; }
model RpcModuleDescriptor extends ModuleDescriptorBase { kind: "rpc"; `interface`: "scintilla.rpc.v1"; }
model WorkerModuleDescriptor extends ModuleDescriptorBase { kind: "worker"; `interface`: "scintilla.worker.v1"; }
model ActorModuleDescriptor extends ModuleDescriptorBase { kind: "actor"; `interface`: "scintilla.actor.v1"; }
model ContainerModuleDescriptor extends ModuleDescriptorBase { kind: "container"; `interface`: "scintilla.container.v1"; }

union ModuleDescriptor {
  lambda: LambdaModuleDescriptor,
  http: HttpModuleDescriptor,
  rpc: RpcModuleDescriptor,
  worker: WorkerModuleDescriptor,
  actor: ActorModuleDescriptor,
  container: ContainerModuleDescriptor,
}
