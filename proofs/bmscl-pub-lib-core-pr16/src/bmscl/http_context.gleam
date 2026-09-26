import bmscl.{
  type ClusterCapability, type HostedCapability, type LogCapability,
  type ModuleKind, type ModulePolicy, type Request, type Response, Http,
  HttpClient, default_policy,
}

/// Opaque outbound-HTTP authority minted only by the trusted Hosted v3 adapter.
/// Hosted v2 handlers continue to use `bmscl.Context` and never receive this.
pub opaque type HttpCapability {
  HttpCapability(token: BitArray)
}

/// Hosted v3 context: read-only same-cluster calls, scoped outbound HTTP,
/// non-authoritative logging, and an invocation deadline.
pub type HttpContext {
  HttpContext(
    cluster: ClusterCapability,
    http: HttpCapability,
    log: LogCapability,
    deadline_unix_ms: Int,
  )
}

/// Canonical Hosted v3 handler shape. Unlike the v2 `bmscl.Module`, this
/// explicitly requires an HttpContext so outbound HTTP authority cannot be
/// accidentally exposed to a handler compiled against the minimal context.
pub type HttpLambdaEntrypoint =
  fn(Request, HttpContext) -> Response

/// Typed v3 module wrapper. The HTTP capability request is explicit metadata;
/// the opaque HttpCapability inside HttpContext remains the actual authority.
pub opaque type HttpModule(input, output) {
  HttpModule(
    kind: ModuleKind,
    policy: ModulePolicy,
    capabilities: List(HostedCapability),
    entrypoint: fn(input, HttpContext) -> output,
  )
}

pub fn module(
  entrypoint: fn(input, HttpContext) -> output,
) -> HttpModule(input, output) {
  HttpModule(
    kind: Http,
    policy: default_policy(Http),
    capabilities: [HttpClient],
    entrypoint: entrypoint,
  )
}

pub fn http(entrypoint: HttpLambdaEntrypoint) -> HttpModule(Request, Response) {
  module(entrypoint)
}

/// Adapt the stable Hosted v3 HttpContext into an application-owned context
/// before invoking user code. This mirrors `bmscl.module_with_context` for the
/// HTTP-capable profile without widening platform authority.
pub fn module_with_context(
  context_builder: fn(HttpContext) -> user_context,
  entrypoint: fn(input, user_context) -> output,
) -> HttpModule(input, output) {
  module(fn(input, platform_context) {
    entrypoint(input, context_builder(platform_context))
  })
}

/// Context-adapting variant for callers that want the canonical HTTP policy
/// and capability declaration to remain explicit in the source API.
pub fn module_with_policy_and_context(
  policy: ModulePolicy,
  capabilities: List(HostedCapability),
  context_builder: fn(HttpContext) -> user_context,
  entrypoint: fn(input, user_context) -> output,
) -> HttpModule(input, output) {
  HttpModule(
    kind: Http,
    policy: policy,
    capabilities: capabilities,
    entrypoint: fn(input, platform_context) {
      entrypoint(input, context_builder(platform_context))
    },
  )
}

/// Convenience accessor so application context builders do not need to
/// destructure the platform context just to preserve deadline metadata.
pub fn deadline_unix_ms(context: HttpContext) -> Int {
  let HttpContext(_, _, _, deadline) = context
  deadline
}

pub fn module_kind(module_export: HttpModule(input, output)) -> ModuleKind {
  let HttpModule(kind, _, _, _) = module_export
  kind
}

pub fn module_policy(module_export: HttpModule(input, output)) -> ModulePolicy {
  let HttpModule(_, policy, _, _) = module_export
  policy
}

pub fn module_capabilities(
  module_export: HttpModule(input, output),
) -> List(HostedCapability) {
  let HttpModule(_, _, capabilities, _) = module_export
  capabilities
}

pub fn invoke_module(
  module_export: HttpModule(input, output),
  input: input,
  context: HttpContext,
) -> output {
  let HttpModule(_, _, _, entrypoint) = module_export
  entrypoint(input, context)
}
