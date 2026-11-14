use crate::{
    ClusterConfig, ControllerConfig, IngressConfig, LoadBalancingStrategy, ProviderHint,
    RouteConfig, TargetConfig, Violation, INGRESS_CONFIG_SCHEMA,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};
use thiserror::Error;

pub const LAMBDA_MANIFEST_SCHEMA: &str = "ores.lambda.manifest/v1";
const MIDDLEWARE_PACKAGE: &str = "oresoftware/ores-middleware";
const MAX_LOCAL_REPLICAS: u16 = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LambdaManifest {
    pub schema_version: String,
    pub repository: LambdaManifestRepository,
    pub middleware: LambdaMiddlewarePolicy,
    pub local_ingress: LocalIngressPolicy,
    pub functions: Vec<LambdaFunctionConfig>,
    #[serde(default)]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LambdaManifestRepository {
    pub organization: String,
    pub repository: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MiddlewareExecution {
    Invocation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LambdaMiddlewarePolicy {
    pub required: bool,
    pub package: String,
    pub config_path: String,
    pub execution: MiddlewareExecution,
    pub fail_closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalIngressPolicy {
    pub enabled: bool,
    pub bind: String,
    pub request_timeout_ms: u64,
    pub max_request_body_bytes: u64,
    pub max_response_body_bytes: u64,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LambdaSourceKind {
    ApiServer,
    WebServer,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LambdaTriggerKind {
    Http,
    Queue,
    Schedule,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LambdaFunctionSource {
    pub kind: LambdaSourceKind,
    pub repository: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LambdaLocalProcess {
    pub working_dir: String,
    pub command: Vec<String>,
    pub bind_env: String,
    pub host: String,
    pub start_port: u16,
    pub replicas: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LambdaHttpIngress {
    pub ingress_path_prefix: String,
    pub methods: Vec<String>,
    #[serde(default)]
    pub hosts: Vec<String>,
    pub priority: i32,
    pub preserve_host: bool,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LambdaFunctionConfig {
    pub id: String,
    pub entrypoint: String,
    pub trigger: LambdaTriggerKind,
    pub middleware_required: bool,
    pub source: LambdaFunctionSource,
    pub local: Option<LambdaLocalProcess>,
    pub http: Option<LambdaHttpIngress>,
    #[serde(default)]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum LambdaManifestError {
    #[error("read lambda manifest {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parse lambda manifest: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("lambda manifest failed validation: {0:?}")]
    Invalid(Vec<Violation>),
}

impl LambdaManifest {
    pub fn from_toml_str(input: &str) -> Result<Self, LambdaManifestError> {
        let manifest: Self = toml::from_str(input)?;
        let violations = manifest.validate();
        if violations.is_empty() {
            Ok(manifest)
        } else {
            Err(LambdaManifestError::Invalid(violations))
        }
    }

    pub fn load(path: &Path) -> Result<Self, LambdaManifestError> {
        let input = fs::read_to_string(path).map_err(|source| LambdaManifestError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&input)
    }

    #[must_use]
    pub fn validate(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        if self.schema_version != LAMBDA_MANIFEST_SCHEMA {
            push(
                &mut violations,
                "schema_version",
                "schema_version",
                format!("expected {LAMBDA_MANIFEST_SCHEMA:?}"),
            );
        }
        if self.repository.organization.trim().is_empty() {
            push(
                &mut violations,
                "repository_identity",
                "repository.organization",
                "organization must not be empty",
            );
        }
        if self.repository.repository.trim().is_empty() {
            push(
                &mut violations,
                "repository_identity",
                "repository.repository",
                "repository must not be empty",
            );
        } else if !self.repository.repository.ends_with("-lambdas")
            && !self.repository.repository.ends_with("-lambda")
        {
            push(
                &mut violations,
                "repository_name",
                "repository.repository",
                "fleet manifest belongs in a *-lambdas or *-lambda repository",
            );
        }

        validate_middleware(&self.middleware, &mut violations);
        validate_local_ingress(&self.local_ingress, &mut violations);

        let mut claimed_ports = BTreeMap::<u16, String>::new();
        if self.local_ingress.enabled {
            if let Ok(bind) = self.local_ingress.bind.parse::<std::net::SocketAddr>() {
                if bind.port() != 0 {
                    claimed_ports.insert(bind.port(), "local_ingress.bind".to_owned());
                }
            }
        }

        let mut ids = BTreeSet::new();
        for (index, function) in self.functions.iter().enumerate() {
            let base = format!("functions[{index}]");
            if function.id.trim().is_empty() {
                push(
                    &mut violations,
                    "function_id",
                    format!("{base}.id"),
                    "function id must not be empty",
                );
            } else if !is_safe_id(&function.id) {
                push(
                    &mut violations,
                    "function_id",
                    format!("{base}.id"),
                    "function id must contain only lowercase ASCII letters, digits, '-' or '_'",
                );
            } else if !ids.insert(function.id.as_str()) {
                push(
                    &mut violations,
                    "duplicate_function",
                    format!("{base}.id"),
                    format!("duplicate function id {:?}", function.id),
                );
            }
            if function.entrypoint.trim().is_empty() {
                push(
                    &mut violations,
                    "entrypoint",
                    format!("{base}.entrypoint"),
                    "entrypoint must not be empty",
                );
            }
            if !function.middleware_required {
                push(
                    &mut violations,
                    "middleware_bypass",
                    format!("{base}.middleware_required"),
                    "every lambda invocation must cross the ores-middleware boundary",
                );
            }
            validate_source(
                &self.repository.repository,
                function,
                &base,
                &mut violations,
            );
            validate_local(function.local.as_ref(), &base, &mut violations);
            validate_local_port_claims(
                function.local.as_ref(),
                &base,
                &mut claimed_ports,
                &mut violations,
            );
            validate_http(function, &base, &mut violations);
        }

        violations.sort();
        violations.dedup();
        violations
    }

    pub fn ingress_config(&self) -> Result<IngressConfig, Vec<Violation>> {
        let violations = self.validate();
        if !violations.is_empty() {
            return Err(violations);
        }

        let mut clusters = Vec::new();
        let mut routes = Vec::new();
        if self.local_ingress.enabled {
            for function in self
                .functions
                .iter()
                .filter(|function| matches!(function.trigger, LambdaTriggerKind::Http))
            {
                let local = function
                    .local
                    .as_ref()
                    .expect("validated HTTP lambda has local process");
                let http = function
                    .http
                    .as_ref()
                    .expect("validated HTTP lambda has HTTP ingress");
                let cluster_id = format!("lambda-{}", function.id);
                let targets = (0..local.replicas)
                    .map(|replica| TargetConfig {
                        id: format!("{}-{}", cluster_id, replica + 1),
                        endpoint: format!(
                            "http://{}:{}",
                            local.host,
                            u32::from(local.start_port) + u32::from(replica)
                        ),
                        weight: 1,
                        enabled: true,
                        provider_hint: Some(ProviderHint::Local),
                        extensions: BTreeMap::new(),
                    })
                    .collect();
                clusters.push(ClusterConfig {
                    id: cluster_id.clone(),
                    strategy: LoadBalancingStrategy::WeightedRoundRobin,
                    targets,
                    extensions: BTreeMap::new(),
                });
                routes.push(RouteConfig {
                    id: function.id.clone(),
                    hosts: http.hosts.clone(),
                    path_prefix: http.ingress_path_prefix.clone(),
                    methods: http.methods.clone(),
                    cluster: cluster_id,
                    priority: http.priority,
                    strip_prefix: true,
                    preserve_host: http.preserve_host,
                    timeout_ms: http.timeout_ms,
                    extensions: BTreeMap::new(),
                });
            }
        }

        let config = IngressConfig {
            schema_version: INGRESS_CONFIG_SCHEMA.to_owned(),
            controller: ControllerConfig {
                bind: self.local_ingress.bind.clone(),
                request_timeout_ms: self.local_ingress.request_timeout_ms,
                max_request_body_bytes: self.local_ingress.max_request_body_bytes,
                max_response_body_bytes: self.local_ingress.max_response_body_bytes,
                max_attempts: self.local_ingress.max_attempts,
                trust_forwarded_headers: false,
                extensions: BTreeMap::new(),
            },
            clusters,
            routes,
            extensions: BTreeMap::new(),
        };
        let violations = config.validate();
        if violations.is_empty() {
            Ok(config)
        } else {
            Err(violations)
        }
    }

    #[must_use]
    pub fn compose_services_fragment(&self, ingress_config_path: &str) -> String {
        let mut output = String::from(
            "# generated by ores-lambda from .ores-lambda.toml; do not hand-edit\nservices:\n",
        );
        let mut dependencies = Vec::new();
        for function in &self.functions {
            let Some(local) = &function.local else {
                continue;
            };
            let service = format!("lambda-{}", function.id);
            dependencies.push(service.clone());
            output.push_str(&format!("  {service}:\n    runtime: host\n"));
            output.push_str(&format!(
                "    working_dir: {}\n",
                yaml_scalar(&local.working_dir)
            ));
            output.push_str(&format!(
                "    command: {}\n",
                serde_json::to_string(&local.command).expect("string vector serializes")
            ));
            output.push_str(&format!("    replicas: {}\n", local.replicas));
            output.push_str("    host_bind:\n");
            output.push_str(&format!("      env: {}\n", yaml_scalar(&local.bind_env)));
            output.push_str(&format!("      host: {}\n", yaml_scalar(&local.host)));
            output.push_str(&format!("      start_port: {}\n", local.start_port));
        }
        if self.local_ingress.enabled {
            output.push_str("  lambda-ingress:\n    runtime: host\n");
            output.push_str(&format!(
                "    command: {}\n",
                serde_json::to_string(&vec!["ores-lambda-ingress", ingress_config_path])
                    .expect("command serializes")
            ));
            if !dependencies.is_empty() {
                output.push_str(&format!(
                    "    depends_on: {}\n",
                    serde_json::to_string(&dependencies).expect("dependencies serialize")
                ));
            }
        }
        output
    }
}

fn validate_middleware(policy: &LambdaMiddlewarePolicy, violations: &mut Vec<Violation>) {
    if !policy.required {
        push(
            violations,
            "middleware_required",
            "middleware.required",
            "ores-middleware is mandatory for fleet lambda repositories",
        );
    }
    if !policy.fail_closed {
        push(
            violations,
            "middleware_fail_closed",
            "middleware.fail_closed",
            "middleware admission must fail closed",
        );
    }
    if policy.package != MIDDLEWARE_PACKAGE {
        push(
            violations,
            "middleware_package",
            "middleware.package",
            format!("expected {MIDDLEWARE_PACKAGE:?}"),
        );
    }
    if policy.config_path != ".ores-mw.toml" {
        push(
            violations,
            "middleware_config",
            "middleware.config_path",
            "canonical middleware config path is .ores-mw.toml",
        );
    }
}

fn validate_local_ingress(policy: &LocalIngressPolicy, violations: &mut Vec<Violation>) {
    if policy.enabled {
        match policy.bind.parse::<std::net::SocketAddr>() {
            Ok(bind) if bind.port() == 0 => push(
                violations,
                "local_ingress_bind",
                "local_ingress.bind",
                "enabled local ingress bind port must be non-zero",
            ),
            Ok(_) => {}
            Err(_) => push(
                violations,
                "local_ingress_bind",
                "local_ingress.bind",
                "enabled local ingress bind must be an IP socket address",
            ),
        }
    }
    if policy.request_timeout_ms == 0
        || policy.max_request_body_bytes == 0
        || policy.max_response_body_bytes == 0
        || policy.max_attempts == 0
    {
        push(
            violations,
            "local_ingress_bounds",
            "local_ingress",
            "timeouts, body limits and max_attempts must be non-zero",
        );
    }
}

fn validate_source(
    owner_repo: &str,
    function: &LambdaFunctionConfig,
    base: &str,
    violations: &mut Vec<Violation>,
) {
    if function.source.repository.trim().is_empty() {
        push(
            violations,
            "source_repository",
            format!("{base}.source.repository"),
            "source repository must not be empty",
        );
    }
    if !safe_relative(&function.source.path) {
        push(
            violations,
            "source_path",
            format!("{base}.source.path"),
            "source path must be a safe relative path without '..'",
        );
    }
    match function.source.kind {
        LambdaSourceKind::ApiServer if !function.source.repository.ends_with("-api-server.rs") => {
            push(
                violations,
                "source_kind",
                format!("{base}.source.repository"),
                "api_server source repository must end with -api-server.rs",
            )
        }
        LambdaSourceKind::WebServer if !function.source.repository.ends_with("-web-server.rs") => {
            push(
                violations,
                "source_kind",
                format!("{base}.source.repository"),
                "web_server source repository must end with -web-server.rs",
            )
        }
        LambdaSourceKind::Custom if function.source.repository != owner_repo => push(
            violations,
            "source_ownership",
            format!("{base}.source.repository"),
            "custom lambdas must be owned by this *-lambdas repository",
        ),
        _ => {}
    }
}

fn validate_local(local: Option<&LambdaLocalProcess>, base: &str, violations: &mut Vec<Violation>) {
    let Some(local) = local else { return };
    if !safe_relative(&local.working_dir) {
        push(
            violations,
            "working_dir",
            format!("{base}.local.working_dir"),
            "working_dir must be a safe relative path",
        );
    }
    if local.command.is_empty() || local.command.iter().any(|arg| arg.is_empty()) {
        push(
            violations,
            "local_command",
            format!("{base}.local.command"),
            "local command must contain non-empty argv entries",
        );
    }
    if local.bind_env.trim().is_empty() {
        push(
            violations,
            "bind_env",
            format!("{base}.local.bind_env"),
            "bind_env must not be empty",
        );
    }
    if local.host.parse::<std::net::IpAddr>().is_err() {
        push(
            violations,
            "local_host",
            format!("{base}.local.host"),
            "local host must be an IP address",
        );
    }
    if local.start_port == 0 || local.replicas == 0 || local.replicas > MAX_LOCAL_REPLICAS {
        push(
            violations,
            "local_replica_range",
            format!("{base}.local"),
            format!("start_port must be non-zero and replicas must be 1..={MAX_LOCAL_REPLICAS}"),
        );
    } else if u32::from(local.start_port) + u32::from(local.replicas) - 1 > u32::from(u16::MAX) {
        push(
            violations,
            "local_port_range",
            format!("{base}.local.start_port"),
            "replica port range exceeds 65535",
        );
    }
}

fn validate_local_port_claims(
    local: Option<&LambdaLocalProcess>,
    base: &str,
    claimed_ports: &mut BTreeMap<u16, String>,
    violations: &mut Vec<Violation>,
) {
    let Some(local) = local else { return };
    if local.start_port == 0 || local.replicas == 0 || local.replicas > MAX_LOCAL_REPLICAS {
        return;
    }
    let end = u32::from(local.start_port) + u32::from(local.replicas) - 1;
    if end > u32::from(u16::MAX) {
        return;
    }
    for port in local.start_port..=end as u16 {
        if let Some(owner) = claimed_ports.get(&port) {
            push(
                violations,
                "local_port_collision",
                format!("{base}.local.start_port"),
                format!("local port {port} is already claimed by {owner}"),
            );
        } else {
            claimed_ports.insert(port, format!("{base}.local"));
        }
    }
}

fn validate_http(function: &LambdaFunctionConfig, base: &str, violations: &mut Vec<Violation>) {
    match function.trigger {
        LambdaTriggerKind::Http => {
            if function.local.is_none() {
                push(
                    violations,
                    "http_local_process",
                    format!("{base}.local"),
                    "HTTP lambdas need a local process for ingress/compose projection",
                );
            }
            let Some(http) = &function.http else {
                push(
                    violations,
                    "http_config",
                    format!("{base}.http"),
                    "HTTP trigger requires an http section",
                );
                return;
            };
            if http.ingress_path_prefix == "/"
                || !http.ingress_path_prefix.starts_with('/')
                || http.ingress_path_prefix.ends_with('/')
            {
                push(violations, "ingress_path_prefix", format!("{base}.http.ingress_path_prefix"), "ingress_path_prefix must be a non-root absolute prefix without a trailing slash");
            }
            if http.methods.is_empty()
                || http
                    .methods
                    .iter()
                    .any(|method| method.is_empty() || method != &method.to_ascii_uppercase())
            {
                push(
                    violations,
                    "http_methods",
                    format!("{base}.http.methods"),
                    "HTTP methods must be non-empty uppercase tokens",
                );
            }
        }
        _ if function.http.is_some() => push(
            violations,
            "http_config",
            format!("{base}.http"),
            "non-HTTP triggers must not declare an http section",
        ),
        _ => {}
    }
}

fn safe_relative(value: &str) -> bool {
    let path = Path::new(value);
    !value.trim().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn is_safe_id(value: &str) -> bool {
    value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
    })
}

fn yaml_scalar(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}

fn push(
    violations: &mut Vec<Violation>,
    code: &str,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    violations.push(Violation {
        code: code.to_owned(),
        path: path.into(),
        message: message.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
schema_version = "ores.lambda.manifest/v1"

[repository]
organization = "acme"
repository = "payments-lambdas"

[middleware]
required = true
package = "oresoftware/ores-middleware"
config_path = ".ores-mw.toml"
execution = "invocation"
fail_closed = true

[local_ingress]
enabled = true
bind = "127.0.0.1:8088"
request_timeout_ms = 10000
max_request_body_bytes = 1048576
max_response_body_bytes = 8388608
max_attempts = 2

[[functions]]
id = "create-order"
entrypoint = "create_order"
trigger = "http"
middleware_required = true
[functions.source]
kind = "api_server"
repository = "payments-api-server.rs"
path = "src/lambdas/create_order.rs"
[functions.local]
working_dir = "payments-lambdas"
command = ["cargo", "run", "--locked", "--features", "http", "--bin", "create-order-http"]
bind_env = "LAMBDAS_BIND"
host = "127.0.0.1"
start_port = 19000
replicas = 2
[functions.http]
ingress_path_prefix = "/__ores/lambda/create-order"
methods = ["POST"]
hosts = []
priority = 100
preserve_host = false
"#;

    #[test]
    fn valid_manifest_compiles_to_ingress() {
        let manifest = LambdaManifest::from_toml_str(VALID).unwrap();
        let ingress = manifest.ingress_config().unwrap();
        assert_eq!(ingress.clusters.len(), 1);
        assert_eq!(ingress.clusters[0].targets.len(), 2);
        assert_eq!(ingress.routes[0].path_prefix, "/__ores/lambda/create-order");
        assert!(ingress.routes[0].strip_prefix);
    }

    #[test]
    fn custom_source_cannot_point_at_another_repository() {
        let input = VALID.replace(
            "kind = \"api_server\"\nrepository = \"payments-api-server.rs\"",
            "kind = \"custom\"\nrepository = \"other-lambdas\"",
        );
        let error = LambdaManifest::from_toml_str(&input).unwrap_err();
        assert!(matches!(error, LambdaManifestError::Invalid(_)));
    }

    #[test]
    fn local_ingress_port_cannot_collide_with_worker() {
        let input = VALID.replace("bind = \"127.0.0.1:8088\"", "bind = \"127.0.0.1:19000\"");
        let error = LambdaManifest::from_toml_str(&input).unwrap_err();
        match error {
            LambdaManifestError::Invalid(violations) => assert!(violations
                .iter()
                .any(|violation| violation.code == "local_port_collision")),
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn worker_port_ranges_cannot_overlap() {
        let second = r#"

[[functions]]
id = "capture-payment"
entrypoint = "capture_payment"
trigger = "http"
middleware_required = true
[functions.source]
kind = "api_server"
repository = "payments-api-server.rs"
path = "src/lambdas/capture_payment.rs"
[functions.local]
working_dir = "payments-lambdas"
command = ["cargo", "run", "--locked", "--features", "http", "--bin", "capture-payment-http"]
bind_env = "LAMBDAS_BIND"
host = "127.0.0.1"
start_port = 19001
replicas = 2
[functions.http]
ingress_path_prefix = "/__ores/lambda/capture-payment"
methods = ["POST"]
hosts = []
priority = 100
preserve_host = false
"#;
        let input = format!("{VALID}{second}");
        let error = LambdaManifest::from_toml_str(&input).unwrap_err();
        match error {
            LambdaManifestError::Invalid(violations) => assert!(violations
                .iter()
                .any(|violation| violation.code == "local_port_collision")),
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn enabled_ingress_rejects_ephemeral_port_zero() {
        let input = VALID.replace("bind = \"127.0.0.1:8088\"", "bind = \"127.0.0.1:0\"");
        let error = LambdaManifest::from_toml_str(&input).unwrap_err();
        match error {
            LambdaManifestError::Invalid(violations) => assert!(violations
                .iter()
                .any(|violation| violation.code == "local_ingress_bind")),
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn compose_projection_uses_same_replica_port_authority() {
        let manifest = LambdaManifest::from_toml_str(VALID).unwrap();
        let fragment = manifest.compose_services_fragment(".ores/generated/lambda-ingress.toml");
        assert!(fragment.contains("start_port: 19000"));
        assert!(fragment.contains("replicas: 2"));
        assert!(fragment.contains("lambda-ingress"));
    }
}
