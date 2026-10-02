use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::SocketAddr,
    path::Path,
};
use thiserror::Error;
use url::Url;

pub const INGRESS_CONFIG_SCHEMA: &str = "ores.lambda.ingress-config/v1";
const MAX_REQUEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RESPONSE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 300_000;
const MAX_ATTEMPTS: u32 = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IngressConfig {
    pub schema_version: String,
    pub controller: ControllerConfig,
    pub clusters: Vec<ClusterConfig>,
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ControllerConfig {
    pub bind: String,
    pub request_timeout_ms: u64,
    pub max_request_body_bytes: u64,
    pub max_response_body_bytes: u64,
    pub max_attempts: u32,
    pub trust_forwarded_headers: bool,
    #[serde(default)]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalancingStrategy {
    WeightedRoundRobin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHint {
    Aws,
    Gcp,
    Azure,
    Vercel,
    Local,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClusterConfig {
    pub id: String,
    pub strategy: LoadBalancingStrategy,
    pub targets: Vec<TargetConfig>,
    #[serde(default)]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub id: String,
    pub endpoint: String,
    pub weight: u32,
    pub enabled: bool,
    pub provider_hint: Option<ProviderHint>,
    #[serde(default)]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    pub id: String,
    pub hosts: Vec<String>,
    pub path_prefix: String,
    pub methods: Vec<String>,
    pub cluster: String,
    pub priority: i32,
    pub strip_prefix: bool,
    pub preserve_host: bool,
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub code: String,
    pub path: String,
    pub message: String,
}

impl Violation {
    fn new(code: &str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("read ingress config {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parse ingress config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("ingress config failed validation: {0:?}")]
    Invalid(Vec<Violation>),
}

impl IngressConfig {
    pub fn from_toml_str(input: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(input)?;
        let violations = config.validate();
        if violations.is_empty() {
            Ok(config)
        } else {
            Err(ConfigError::Invalid(violations))
        }
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let input = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&input)
    }

    #[must_use]
    pub fn validate(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        if self.schema_version != INGRESS_CONFIG_SCHEMA {
            violations.push(Violation::new(
                "schema_version",
                "schema_version",
                format!("expected {INGRESS_CONFIG_SCHEMA:?}"),
            ));
        }
        validate_controller(&self.controller, &mut violations);

        let mut cluster_ids = BTreeSet::new();
        for (cluster_index, cluster) in self.clusters.iter().enumerate() {
            let base = format!("clusters[{cluster_index}]");
            if cluster.id.trim().is_empty() {
                violations.push(Violation::new(
                    "cluster_id",
                    format!("{base}.id"),
                    "cluster id is empty",
                ));
            }
            if !cluster_ids.insert(cluster.id.as_str()) {
                violations.push(Violation::new(
                    "duplicate_cluster",
                    format!("{base}.id"),
                    format!("duplicate cluster id {:?}", cluster.id),
                ));
            }
            validate_cluster(cluster, &base, &mut violations);
        }

        let cluster_map = self
            .clusters
            .iter()
            .map(|cluster| (cluster.id.as_str(), cluster))
            .collect::<BTreeMap<_, _>>();
        let mut route_ids = BTreeSet::new();
        for (route_index, route) in self.routes.iter().enumerate() {
            let base = format!("routes[{route_index}]");
            if route.id.trim().is_empty() {
                violations.push(Violation::new(
                    "route_id",
                    format!("{base}.id"),
                    "route id is empty",
                ));
            }
            if !route_ids.insert(route.id.as_str()) {
                violations.push(Violation::new(
                    "duplicate_route",
                    format!("{base}.id"),
                    format!("duplicate route id {:?}", route.id),
                ));
            }
            validate_route(route, &base, &cluster_map, &mut violations);
        }
        validate_route_ambiguity(&self.routes, &mut violations);

        violations.sort();
        violations.dedup();
        violations
    }

    #[must_use]
    pub fn cluster(&self, id: &str) -> Option<&ClusterConfig> {
        self.clusters.iter().find(|cluster| cluster.id == id)
    }
}

fn validate_controller(controller: &ControllerConfig, violations: &mut Vec<Violation>) {
    if controller.bind.parse::<SocketAddr>().is_err() {
        violations.push(Violation::new(
            "bind",
            "controller.bind",
            "bind must be an IP socket address such as 0.0.0.0:8080",
        ));
    }
    if !(1..=MAX_TIMEOUT_MS).contains(&controller.request_timeout_ms) {
        violations.push(Violation::new(
            "request_timeout",
            "controller.request_timeout_ms",
            format!("must be between 1 and {MAX_TIMEOUT_MS}"),
        ));
    }
    if !(1..=MAX_REQUEST_BYTES).contains(&controller.max_request_body_bytes) {
        violations.push(Violation::new(
            "request_body_limit",
            "controller.max_request_body_bytes",
            format!("must be between 1 and {MAX_REQUEST_BYTES}"),
        ));
    }
    if !(1..=MAX_RESPONSE_BYTES).contains(&controller.max_response_body_bytes) {
        violations.push(Violation::new(
            "response_body_limit",
            "controller.max_response_body_bytes",
            format!("must be between 1 and {MAX_RESPONSE_BYTES}"),
        ));
    }
    if !(1..=MAX_ATTEMPTS).contains(&controller.max_attempts) {
        violations.push(Violation::new(
            "max_attempts",
            "controller.max_attempts",
            format!("must be between 1 and {MAX_ATTEMPTS}"),
        ));
    }
    if controller.trust_forwarded_headers {
        violations.push(Violation::new(
            "forwarded_headers_trust",
            "controller.trust_forwarded_headers",
            "true is unsupported until an authenticated trusted-peer/CIDR policy establishes forwarding provenance",
        ));
    }
}

fn validate_cluster(cluster: &ClusterConfig, base: &str, violations: &mut Vec<Violation>) {
    let mut target_ids = BTreeSet::new();
    let mut enabled = 0_u32;
    for (target_index, target) in cluster.targets.iter().enumerate() {
        let path = format!("{base}.targets[{target_index}]");
        if target.id.trim().is_empty() {
            violations.push(Violation::new(
                "target_id",
                format!("{path}.id"),
                "target id is empty",
            ));
        }
        if !target_ids.insert(target.id.as_str()) {
            violations.push(Violation::new(
                "duplicate_target",
                format!("{path}.id"),
                format!("duplicate target id {:?}", target.id),
            ));
        }
        if target.enabled {
            enabled += 1;
            if target.weight == 0 {
                violations.push(Violation::new(
                    "target_weight",
                    format!("{path}.weight"),
                    "enabled targets require weight > 0",
                ));
            }
        }
        match Url::parse(&target.endpoint) {
            Ok(url) => {
                if !matches!(url.scheme(), "http" | "https") {
                    violations.push(Violation::new(
                        "target_scheme",
                        format!("{path}.endpoint"),
                        "target endpoint scheme must be http or https",
                    ));
                }
                if url.host_str().is_none() {
                    violations.push(Violation::new(
                        "target_host",
                        format!("{path}.endpoint"),
                        "target endpoint requires a host",
                    ));
                }
                if !url.username().is_empty() || url.password().is_some() {
                    violations.push(Violation::new(
                        "target_credentials",
                        format!("{path}.endpoint"),
                        "target endpoint must not embed credentials",
                    ));
                }
                if url.query().is_some() || url.fragment().is_some() {
                    violations.push(Violation::new(
                        "target_endpoint_shape",
                        format!("{path}.endpoint"),
                        "target endpoint must not contain query or fragment components",
                    ));
                }
            }
            Err(error) => violations.push(Violation::new(
                "target_url",
                format!("{path}.endpoint"),
                format!("invalid target URL: {error}"),
            )),
        }
    }
    if enabled == 0 {
        violations.push(Violation::new(
            "cluster_targets",
            format!("{base}.targets"),
            "cluster requires at least one enabled target",
        ));
    }
}

fn validate_route(
    route: &RouteConfig,
    base: &str,
    clusters: &BTreeMap<&str, &ClusterConfig>,
    violations: &mut Vec<Violation>,
) {
    if !clusters.contains_key(route.cluster.as_str()) {
        violations.push(Violation::new(
            "unknown_cluster",
            format!("{base}.cluster"),
            format!("unknown cluster {:?}", route.cluster),
        ));
    }
    if !valid_path_prefix(&route.path_prefix) {
        violations.push(Violation::new(
            "path_prefix",
            format!("{base}.path_prefix"),
            "path_prefix must be normalized, absolute, query-free, fragment-free, and contain no dot segments",
        ));
    }
    if reserved_control_path(&route.path_prefix) {
        violations.push(Violation::new(
            "reserved_path",
            format!("{base}.path_prefix"),
            "/__ores control paths are reserved; fleet Lambda data-plane routes belong under /__ores/lambda/<function>",
        ));
    }
    if route.methods.is_empty() {
        violations.push(Violation::new(
            "methods",
            format!("{base}.methods"),
            "route requires at least one method",
        ));
    }
    let mut methods = BTreeSet::new();
    for (index, method) in route.methods.iter().enumerate() {
        if !valid_method(method) {
            violations.push(Violation::new(
                "method",
                format!("{base}.methods[{index}]"),
                "method must be a non-empty uppercase HTTP token",
            ));
        }
        if !methods.insert(method.as_str()) {
            violations.push(Violation::new(
                "duplicate_method",
                format!("{base}.methods[{index}]"),
                format!("duplicate method {method:?}"),
            ));
        }
    }
    let mut hosts = BTreeSet::new();
    for (index, host) in route.hosts.iter().enumerate() {
        if host.trim().is_empty()
            || host.contains("//")
            || host.contains('/')
            || host.chars().any(char::is_whitespace)
        {
            violations.push(Violation::new(
                "host",
                format!("{base}.hosts[{index}]"),
                "host must be an exact authority/host value without scheme, path, or whitespace",
            ));
        }
        let normalized = host.to_ascii_lowercase();
        if !hosts.insert(normalized) {
            violations.push(Violation::new(
                "duplicate_host",
                format!("{base}.hosts[{index}]"),
                format!("duplicate host {host:?}"),
            ));
        }
    }
    if let Some(timeout) = route.timeout_ms {
        if !(1..=MAX_TIMEOUT_MS).contains(&timeout) {
            violations.push(Violation::new(
                "route_timeout",
                format!("{base}.timeout_ms"),
                format!("must be between 1 and {MAX_TIMEOUT_MS}"),
            ));
        }
    }
}

fn validate_route_ambiguity(routes: &[RouteConfig], violations: &mut Vec<Violation>) {
    for left_index in 0..routes.len() {
        for right_index in (left_index + 1)..routes.len() {
            let left = &routes[left_index];
            let right = &routes[right_index];
            if left.priority == right.priority
                && left.path_prefix == right.path_prefix
                && hosts_overlap(&left.hosts, &right.hosts)
                && methods_overlap(&left.methods, &right.methods)
            {
                violations.push(Violation::new(
                    "ambiguous_route",
                    format!("routes[{left_index}]|routes[{right_index}]"),
                    format!(
                        "routes {:?} and {:?} have the same priority/path and overlapping hosts/methods",
                        left.id, right.id
                    ),
                ));
            }
        }
    }
}

fn valid_path_prefix(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('?')
        && !path.contains('#')
        && !path.contains('\0')
        && !path.contains("//")
        && !path.split('/').any(|segment| matches!(segment, "." | ".."))
}

fn reserved_control_path(path: &str) -> bool {
    path == "/__ores"
        || path == "/__ores/lambda"
        || (path.starts_with("/__ores/") && !path.starts_with("/__ores/lambda/"))
}

fn valid_method(method: &str) -> bool {
    !method.is_empty()
        && method
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
}

fn hosts_overlap(left: &[String], right: &[String]) -> bool {
    left.is_empty()
        || right.is_empty()
        || left
            .iter()
            .any(|a| right.iter().any(|b| a.eq_ignore_ascii_case(b)))
}

fn methods_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|a| right.iter().any(|b| a == b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route_fixture(path_prefix: &str) -> String {
        format!(
            r#"
schema_version = "ores.lambda.ingress-config/v1"
[controller]
bind = "127.0.0.1:8080"
request_timeout_ms = 1000
max_request_body_bytes = 1024
max_response_body_bytes = 2048
max_attempts = 1
trust_forwarded_headers = false
[[clusters]]
id = "c"
strategy = "weighted_round_robin"
[[clusters.targets]]
id = "t"
endpoint = "http://127.0.0.1:9000"
weight = 1
enabled = true
[[routes]]
id = "a"
hosts = []
path_prefix = "{path_prefix}"
methods = ["POST"]
cluster = "c"
priority = 1
strip_prefix = true
preserve_host = false
"#
        )
    }

    #[test]
    fn invalid_duplicate_route_is_rejected() {
        let input = r#"
schema_version = "ores.lambda.ingress-config/v1"
[controller]
bind = "127.0.0.1:8080"
request_timeout_ms = 1000
max_request_body_bytes = 1024
max_response_body_bytes = 2048
max_attempts = 1
trust_forwarded_headers = false
[[clusters]]
id = "c"
strategy = "weighted_round_robin"
[[clusters.targets]]
id = "t"
endpoint = "http://127.0.0.1:9000"
weight = 1
enabled = true
[[routes]]
id = "a"
hosts = []
path_prefix = "/x"
methods = ["GET"]
cluster = "c"
priority = 1
strip_prefix = false
preserve_host = false
[[routes]]
id = "b"
hosts = []
path_prefix = "/x"
methods = ["GET"]
cluster = "c"
priority = 1
strip_prefix = false
preserve_host = false
"#;
        let error = IngressConfig::from_toml_str(input).unwrap_err();
        assert!(
            matches!(error, ConfigError::Invalid(items) if items.iter().any(|item| item.code == "ambiguous_route"))
        );
    }

    #[test]
    fn unproven_forwarded_header_trust_is_rejected_by_shared_admission() {
        let input = route_fixture("/forwarded-header-test").replace(
            "trust_forwarded_headers = false",
            "trust_forwarded_headers = true",
        );
        let error = IngressConfig::from_toml_str(&input).unwrap_err();
        assert!(
            matches!(error, ConfigError::Invalid(items) if items.iter().any(|item| item.code == "forwarded_headers_trust"))
        );
    }

    #[test]
    fn fleet_lambda_data_plane_namespace_is_admitted() {
        let input = route_fixture("/__ores/lambda/create-order");
        IngressConfig::from_toml_str(&input).expect("fleet lambda path must remain usable");
    }

    #[test]
    fn ingress_control_namespace_remains_reserved() {
        for path in ["/__ores", "/__ores/health", "/__ores/control/future"] {
            let input = route_fixture(path);
            let error = IngressConfig::from_toml_str(&input).unwrap_err();
            assert!(
                matches!(error, ConfigError::Invalid(items) if items.iter().any(|item| item.code == "reserved_path"))
            );
        }
    }
}
