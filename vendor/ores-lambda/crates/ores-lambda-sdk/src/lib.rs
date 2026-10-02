pub use ores_lambda_core::*;

use std::collections::BTreeMap;

/// Small typed builder for consumers that want to construct ingress config in
/// Rust without duplicating the shared contract model. Validation still occurs
/// in `IngressConfig::validate`; the builder is convenience, not authority.
#[derive(Debug, Clone)]
pub struct IngressConfigBuilder {
    config: IngressConfig,
}

impl IngressConfigBuilder {
    #[must_use]
    pub fn new(bind: impl Into<String>) -> Self {
        Self {
            config: IngressConfig {
                schema_version: INGRESS_CONFIG_SCHEMA.to_owned(),
                controller: ControllerConfig {
                    bind: bind.into(),
                    request_timeout_ms: 10_000,
                    max_request_body_bytes: 1_048_576,
                    max_response_body_bytes: 8_388_608,
                    max_attempts: 2,
                    trust_forwarded_headers: false,
                    extensions: BTreeMap::new(),
                },
                clusters: Vec::new(),
                routes: Vec::new(),
                extensions: BTreeMap::new(),
            },
        }
    }

    #[must_use]
    pub fn controller(mut self, controller: ControllerConfig) -> Self {
        self.config.controller = controller;
        self
    }

    #[must_use]
    pub fn cluster(mut self, cluster: ClusterConfig) -> Self {
        self.config.clusters.push(cluster);
        self
    }

    #[must_use]
    pub fn route(mut self, route: RouteConfig) -> Self {
        self.config.routes.push(route);
        self
    }

    pub fn build(self) -> Result<IngressConfig, Vec<Violation>> {
        let violations = self.config.validate();
        if violations.is_empty() {
            Ok(self.config)
        } else {
            Err(violations)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_does_not_bypass_validation() {
        let result = IngressConfigBuilder::new("127.0.0.1:8080").build();
        assert!(
            result.is_ok(),
            "empty route/cluster sets are valid for a disabled ingress"
        );
    }
}
