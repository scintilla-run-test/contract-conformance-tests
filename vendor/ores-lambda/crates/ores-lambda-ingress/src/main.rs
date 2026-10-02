use anyhow::{bail, Context};
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{
        header, HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use ores_lambda_core::{
    match_route, rewrite_path, select_target, ClusterConfig, IngressConfig, IngressReceipt,
    RequestKey, RouteConfig, TargetConfig,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tracing::{info, warn};

const CONTROL_HEALTH_PATH: &str = "/__ores/health";

#[derive(Clone)]
struct AppState {
    config: Arc<IngressConfig>,
    client: reqwest::Client,
    cursors: Arc<BTreeMap<String, AtomicU64>>,
}

#[derive(Debug)]
enum ForwardError {
    Transport,
    Timeout,
    ResponseTooLarge,
    InvalidUpstreamResponse,
}

struct ForwardedResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ores_lambda_ingress=info".into()),
        )
        .init();

    let mut args = env::args().skip(1).collect::<Vec<_>>();
    let check_only = matches!(args.first().map(String::as_str), Some("--check-config"));
    if check_only {
        args.remove(0);
    }
    if args.len() > 1 {
        bail!("usage: ores-lambda-ingress [--check-config] [CONFIG_PATH]");
    }
    let config_path = args
        .first()
        .map(PathBuf::from)
        .or_else(|| env::var_os("ORES_LAMBDA_CONFIG").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(".ores-lambda.toml"));
    let config = IngressConfig::load(&config_path)
        .with_context(|| format!("admit ingress config {}", config_path.display()))?;

    // A boolean cannot establish trusted proxy provenance. Until the config has
    // a concrete admitted peer identity/CIDR policy, accepting forwarded headers
    // would let a directly connected caller manufacture upstream provenance.
    // Refuse both startup and --check-config rather than silently trusting them.
    if config.controller.trust_forwarded_headers {
        bail!(
            "controller.trust_forwarded_headers=true is unsupported without an authenticated trusted-peer policy; caller forwarding headers remain untrusted"
        );
    }

    if check_only {
        let receipt = IngressReceipt::from_config(&config)?;
        println!("{}", serde_json::to_string_pretty(&receipt)?);
        return Ok(());
    }

    let bind = config.controller.bind.clone();
    let cursors = config
        .clusters
        .iter()
        .map(|cluster| (cluster.id.clone(), AtomicU64::new(0)))
        .collect::<BTreeMap<_, _>>();
    let state = AppState {
        config: Arc::new(config),
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("build ingress HTTP client")?,
        cursors: Arc::new(cursors),
    };
    let app = Router::new()
        .route(CONTROL_HEALTH_PATH, get(health))
        .fallback(proxy)
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind ingress listener {bind}"))?;
    info!(bind = %bind, "ores-lambda ingress listening");
    axum::serve(listener, app).await.context("serve ingress")
}

async fn health(State(state): State<AppState>) -> Response<Body> {
    match IngressReceipt::from_config(&state.config) {
        Ok(receipt) => Json(receipt).into_response(),
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "ingress receipt failed"),
    }
}

async fn proxy(State(state): State<AppState>, request: Request<Body>) -> Response<Body> {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    if connection_tokens(&headers).is_err() {
        return plain(StatusCode::BAD_REQUEST, "invalid connection header");
    }
    let host = match request_host(&headers) {
        Ok(host) => host,
        Err(()) => return plain(StatusCode::BAD_REQUEST, "invalid host header"),
    };
    let route = match match_route(
        &state.config,
        RequestKey {
            method: method.as_str(),
            host: host.as_deref(),
            path: uri.path(),
        },
    ) {
        Ok(Some(route)) => route,
        Ok(None) => return plain(StatusCode::NOT_FOUND, "no ingress route"),
        Err(_) => return plain(StatusCode::INTERNAL_SERVER_ERROR, "ambiguous ingress route"),
    };
    let cluster = match state.config.cluster(&route.cluster) {
        Some(cluster) => cluster,
        None => {
            return plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "ingress cluster unavailable",
            )
        }
    };

    let limit =
        usize::try_from(state.config.controller.max_request_body_bytes).unwrap_or(usize::MAX);
    let body = match to_bytes(request.into_body(), limit).await {
        Ok(body) => body.to_vec(),
        Err(_) => return plain(StatusCode::PAYLOAD_TOO_LARGE, "request body too large"),
    };

    let cursor = state
        .cursors
        .get(&cluster.id)
        .map_or(0, |cursor| cursor.fetch_add(1, Ordering::Relaxed));
    let candidates = target_candidates(
        cluster,
        cursor,
        usize::try_from(state.config.controller.max_attempts).unwrap_or(1),
    );
    if candidates.is_empty() {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "no enabled ingress target");
    }

    let retryable_method = matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);
    for (attempt, target) in candidates.iter().enumerate() {
        let result = forward_once(
            &state,
            route,
            target,
            ForwardInput {
                method: &method,
                uri: &uri,
                headers: &headers,
                original_host: host.as_deref(),
                body: &body,
            },
        )
        .await;
        match result {
            Ok(response) => {
                if retryable_method
                    && attempt + 1 < candidates.len()
                    && matches!(
                        response.status,
                        StatusCode::BAD_GATEWAY
                            | StatusCode::SERVICE_UNAVAILABLE
                            | StatusCode::GATEWAY_TIMEOUT
                    )
                {
                    warn!(route = %route.id, target = %target.id, status = %response.status, "retrying idempotent ingress request");
                    continue;
                }
                return into_axum_response(response);
            }
            Err(error) => {
                warn!(route = %route.id, target = %target.id, ?error, "ingress target attempt failed");
                if retryable_method && attempt + 1 < candidates.len() {
                    continue;
                }
                let status = if matches!(error, ForwardError::Timeout) {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                };
                return plain(status, "upstream invocation failed");
            }
        }
    }
    plain(StatusCode::BAD_GATEWAY, "upstream invocation failed")
}

struct ForwardInput<'a> {
    method: &'a Method,
    uri: &'a Uri,
    headers: &'a HeaderMap,
    original_host: Option<&'a str>,
    body: &'a [u8],
}

async fn forward_once(
    state: &AppState,
    route: &RouteConfig,
    target: &TargetConfig,
    input: ForwardInput<'_>,
) -> Result<ForwardedResponse, ForwardError> {
    let path = rewrite_path(route, input.uri.path());
    let upstream = upstream_url(&target.endpoint, &path, input.uri.query());
    let timeout_ms = route
        .timeout_ms
        .unwrap_or(state.config.controller.request_timeout_ms);
    let connection_tokens =
        connection_tokens(input.headers).map_err(|()| ForwardError::Transport)?;
    let future = async {
        let request_method = reqwest::Method::from_bytes(input.method.as_str().as_bytes())
            .map_err(|_| ForwardError::Transport)?;
        let mut builder = state
            .client
            .request(request_method, upstream)
            .body(input.body.to_vec());
        for (name, value) in input.headers {
            if should_strip_request_header(name.as_str(), route.preserve_host, &connection_tokens) {
                continue;
            }
            builder = builder.header(name.as_str(), value.as_bytes());
        }
        if route.preserve_host {
            if let Some(host) = input.original_host {
                builder = builder.header(header::HOST.as_str(), host);
            }
        }
        builder = builder
            .header("x-ores-ingress-route", &route.id)
            .header("x-ores-ingress-cluster", &route.cluster)
            .header("x-ores-ingress-target", &target.id);

        let mut response = builder.send().await.map_err(|_| ForwardError::Transport)?;
        let status = StatusCode::from_u16(response.status().as_u16())
            .map_err(|_| ForwardError::InvalidUpstreamResponse)?;
        let headers = response.headers().clone();
        let max =
            usize::try_from(state.config.controller.max_response_body_bytes).unwrap_or(usize::MAX);
        let mut response_body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ForwardError::Transport)?
        {
            if response_body.len().saturating_add(chunk.len()) > max {
                return Err(ForwardError::ResponseTooLarge);
            }
            response_body.extend_from_slice(&chunk);
        }
        Ok(ForwardedResponse {
            status,
            headers,
            body: response_body,
        })
    };
    tokio::time::timeout(Duration::from_millis(timeout_ms), future)
        .await
        .map_err(|_| ForwardError::Timeout)?
}

fn target_candidates(
    cluster: &ClusterConfig,
    cursor: u64,
    max_attempts: usize,
) -> Vec<&TargetConfig> {
    let enabled = cluster
        .targets
        .iter()
        .filter(|target| target.enabled && target.weight > 0)
        .collect::<Vec<_>>();
    if enabled.is_empty() || max_attempts == 0 {
        return Vec::new();
    }
    let Some(first) = select_target(cluster, cursor) else {
        return Vec::new();
    };
    let first_index = enabled
        .iter()
        .position(|target| target.id == first.id)
        .unwrap_or(0);
    (0..enabled.len().min(max_attempts))
        .map(|offset| enabled[(first_index + offset) % enabled.len()])
        .collect()
}

fn upstream_url(endpoint: &str, path: &str, query: Option<&str>) -> String {
    let mut value = format!("{}{}", endpoint.trim_end_matches('/'), path);
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        value.push('?');
        value.push_str(query);
    }
    value
}

fn request_host(headers: &HeaderMap) -> Result<Option<String>, ()> {
    let Some(value) = headers.get(header::HOST) else {
        return Ok(None);
    };
    let host = value.to_str().map_err(|_| ())?;
    Ok(Some(host.to_ascii_lowercase()))
}

/// Parse the Connection nominated-header list. An unparsable value or token is
/// itself invalid authority: silently ignoring it would let a nominated
/// hop-by-hop header cross the proxy boundary.
fn connection_tokens(headers: &HeaderMap) -> Result<BTreeSet<String>, ()> {
    let mut tokens = BTreeSet::new();
    for value in headers.get_all(header::CONNECTION) {
        let value = value.to_str().map_err(|_| ())?;
        for token in value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            HeaderName::from_bytes(token.as_bytes()).map_err(|_| ())?;
            tokens.insert(token.to_ascii_lowercase());
        }
    }
    Ok(tokens)
}

fn should_strip_request_header(
    name: &str,
    preserve_host: bool,
    connection_tokens: &BTreeSet<String>,
) -> bool {
    let lower = name.to_ascii_lowercase();
    is_hop_by_hop(&lower)
        || connection_tokens.contains(&lower)
        || lower == "content-length"
        || (!preserve_host && lower == "host")
        || lower.starts_with("x-ores-ingress-")
        // No trusted-peer policy exists yet, so the entire conventional
        // forwarding namespace is caller-controlled authority. Enumerating only
        // today's common names would let a future/less-common field such as
        // x-forwarded-client-cert cross the trust boundary.
        || lower == "forwarded"
        || lower.starts_with("x-forwarded-")
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn into_axum_response(response: ForwardedResponse) -> Response<Body> {
    let connection_tokens = match connection_tokens(&response.headers) {
        Ok(tokens) => tokens,
        Err(()) => {
            return plain(
                StatusCode::BAD_GATEWAY,
                "invalid upstream connection header",
            )
        }
    };
    let mut builder = Response::builder().status(response.status);
    if let Some(output) = builder.headers_mut() {
        for (name, value) in &response.headers {
            let lower = name.as_str().to_ascii_lowercase();
            if is_hop_by_hop(&lower)
                || connection_tokens.contains(&lower)
                || lower == "content-length"
                || lower == "set-cookie"
            {
                continue;
            }
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_str().as_bytes()),
                HeaderValue::from_bytes(value.as_bytes()),
            ) {
                output.append(name, value);
            }
        }
        if !connection_tokens.contains("set-cookie") {
            for value in response.headers.get_all(header::SET_COOKIE).iter() {
                if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) {
                    output.append(header::SET_COOKIE, value);
                }
            }
        }
    }
    builder
        .body(Body::from(response.body))
        .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "response build failed"))
}

fn plain(status: StatusCode, message: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(message))
        .expect("static ingress response is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ores_lambda_core::{LoadBalancingStrategy, TargetConfig};
    use std::collections::BTreeMap;

    fn target(id: &str, weight: u32) -> TargetConfig {
        TargetConfig {
            id: id.to_owned(),
            endpoint: format!("http://{id}.example.test"),
            weight,
            enabled: true,
            provider_hint: None,
            extensions: BTreeMap::new(),
        }
    }

    #[test]
    fn retry_candidates_are_distinct_even_when_first_target_is_heavy() {
        let cluster = ClusterConfig {
            id: "c".to_owned(),
            strategy: LoadBalancingStrategy::WeightedRoundRobin,
            targets: vec![target("a", 100), target("b", 1), target("c", 1)],
            extensions: BTreeMap::new(),
        };
        let ids = target_candidates(&cluster, 0, 3)
            .into_iter()
            .map(|target| target.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn client_ingress_forwarded_and_connection_nominated_headers_are_stripped() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, HeaderValue::from_static("x-remove-me"));
        headers.insert("x-remove-me", HeaderValue::from_static("secret"));
        let tokens = connection_tokens(&headers).expect("valid Connection token list");
        assert!(should_strip_request_header(
            "x-ores-ingress-target",
            false,
            &tokens
        ));
        assert!(should_strip_request_header("x-remove-me", false, &tokens));
        assert!(should_strip_request_header(
            "x-forwarded-for",
            false,
            &tokens
        ));
        assert!(should_strip_request_header(
            "x-forwarded-client-cert",
            false,
            &tokens
        ));
        assert!(should_strip_request_header(
            "x-forwarded-any-future-field",
            false,
            &tokens
        ));
        assert!(should_strip_request_header("forwarded", false, &tokens));
        assert!(!should_strip_request_header(
            "x-forward-looking",
            false,
            &tokens
        ));
    }

    #[test]
    fn malformed_connection_authority_is_refused_not_ignored() {
        let mut non_ascii = HeaderMap::new();
        non_ascii.insert(
            header::CONNECTION,
            HeaderValue::from_bytes(b"x-remove-me,\x80").expect("obs-text header value"),
        );
        assert!(connection_tokens(&non_ascii).is_err());

        let mut invalid_token = HeaderMap::new();
        invalid_token.insert(
            header::CONNECTION,
            HeaderValue::from_static("x-remove-me, bad token"),
        );
        assert!(connection_tokens(&invalid_token).is_err());
    }

    #[test]
    fn malformed_upstream_connection_metadata_becomes_bad_gateway() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_bytes(b"x-remove-me,\x80").expect("obs-text header value"),
        );
        headers.insert("x-remove-me", HeaderValue::from_static("secret"));
        let response = into_axum_response(ForwardedResponse {
            status: StatusCode::OK,
            headers,
            body: b"ok".to_vec(),
        });
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(!response.headers().contains_key("x-remove-me"));
    }

    #[test]
    fn upstream_url_preserves_raw_query() {
        assert_eq!(
            upstream_url("https://example.test/base/", "/v1/a%2Fb", Some("x=1%202")),
            "https://example.test/base/v1/a%2Fb?x=1%202"
        );
    }
}
