use crate::{
    IngressConfig, LambdaRepositoryDescriptor, Violation, INGRESS_CONFIG_SCHEMA,
    LAMBDA_REPOSITORY_SCHEMA,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const INGRESS_RECEIPT_SCHEMA: &str = "ores.lambda.ingress-receipt/v1";
pub const CONFORMANCE_RECEIPT_SCHEMA: &str = "ores.lambda.conformance-receipt/v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngressReceipt {
    pub schema_version: String,
    pub config_sha256: String,
    pub route_count: u32,
    pub cluster_count: u32,
    pub target_count: u32,
    pub valid: bool,
    pub violations: Vec<Violation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConformanceReceipt {
    pub schema_version: String,
    pub subject_kind: String,
    pub subject_sha256: String,
    pub contract_id: String,
    pub passed: bool,
    pub violations: Vec<Violation>,
}

impl IngressReceipt {
    pub fn from_config(config: &IngressConfig) -> Result<Self, serde_json::Error> {
        let mut violations = config.validate();
        violations.sort();
        let target_count = config
            .clusters
            .iter()
            .map(|cluster| cluster.targets.len())
            .sum::<usize>();
        Ok(Self {
            schema_version: INGRESS_RECEIPT_SCHEMA.to_owned(),
            config_sha256: sha256_json(config)?,
            route_count: u32::try_from(config.routes.len()).unwrap_or(u32::MAX),
            cluster_count: u32::try_from(config.clusters.len()).unwrap_or(u32::MAX),
            target_count: u32::try_from(target_count).unwrap_or(u32::MAX),
            valid: violations.is_empty(),
            violations,
        })
    }
}

impl ConformanceReceipt {
    pub fn ingress(config: &IngressConfig) -> Result<Self, serde_json::Error> {
        let mut violations = config.validate();
        violations.sort();
        Ok(Self {
            schema_version: CONFORMANCE_RECEIPT_SCHEMA.to_owned(),
            subject_kind: "ingress_config".to_owned(),
            subject_sha256: sha256_json(config)?,
            contract_id: INGRESS_CONFIG_SCHEMA.to_owned(),
            passed: violations.is_empty(),
            violations,
        })
    }

    pub fn repository(descriptor: &LambdaRepositoryDescriptor) -> Result<Self, serde_json::Error> {
        let mut violations = descriptor.validate();
        violations.sort();
        Ok(Self {
            schema_version: CONFORMANCE_RECEIPT_SCHEMA.to_owned(),
            subject_kind: "lambda_repository".to_owned(),
            subject_sha256: sha256_json(descriptor)?,
            contract_id: LAMBDA_REPOSITORY_SCHEMA.to_owned(),
            passed: violations.is_empty(),
            violations,
        })
    }
}

pub fn sha256_json(value: &impl Serialize) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(value)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_are_timestamp_free_and_deterministic() {
        let config = IngressConfig::from_toml_str(
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
id = "r"
hosts = []
path_prefix = "/"
methods = ["GET"]
cluster = "c"
priority = 1
strip_prefix = false
preserve_host = false
"#,
        )
        .unwrap();
        let a = IngressReceipt::from_config(&config).unwrap();
        let b = IngressReceipt::from_config(&config).unwrap();
        assert_eq!(a, b);
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("time"));
    }
}
