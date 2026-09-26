import bmscl.{type Request, type Response, Http}
import bmscl/http_context.{type HttpContext}

pub type AppContext {
  AppContext(deadline_unix_ms: Int)
}

fn build_context(ctx: HttpContext) -> AppContext {
  AppContext(http_context.deadline_unix_ms(ctx))
}

fn handle(_request: Request, _ctx: AppContext) -> Response {
  bmscl.text(200, "ok")
}

pub fn adapted_http_module() {
  http_context.module_with_context(build_context, handle)
}

pub fn adapted_http_module_with_policy() {
  http_context.module_with_policy_and_context(
    bmscl.default_policy(Http),
    [bmscl.HttpClient],
    build_context,
    handle,
  )
}
