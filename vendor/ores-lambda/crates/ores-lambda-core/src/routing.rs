use crate::{ClusterConfig, IngressConfig, RouteConfig, TargetConfig};
use std::cmp::Reverse;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct RequestKey<'a> {
    pub method: &'a str,
    pub host: Option<&'a str>,
    pub path: &'a str,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RouteMatchError {
    #[error("ambiguous best route match: {route_ids:?}")]
    Ambiguous { route_ids: Vec<String> },
}

pub fn match_route<'a>(
    config: &'a IngressConfig,
    request: RequestKey<'_>,
) -> Result<Option<&'a RouteConfig>, RouteMatchError> {
    let mut matches = config
        .routes
        .iter()
        .filter(|route| route_matches(route, request))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Ok(None);
    }
    matches.sort_by_key(|route| (Reverse(route.priority), Reverse(route.path_prefix.len())));
    let best_priority = matches[0].priority;
    let best_len = matches[0].path_prefix.len();
    let best = matches
        .into_iter()
        .take_while(|route| route.priority == best_priority && route.path_prefix.len() == best_len)
        .collect::<Vec<_>>();
    if best.len() > 1 {
        let mut route_ids = best
            .iter()
            .map(|route| route.id.clone())
            .collect::<Vec<_>>();
        route_ids.sort();
        return Err(RouteMatchError::Ambiguous { route_ids });
    }
    Ok(best.into_iter().next())
}

#[must_use]
pub fn select_target(cluster: &ClusterConfig, cursor: u64) -> Option<&TargetConfig> {
    let total = cluster
        .targets
        .iter()
        .filter(|target| target.enabled && target.weight > 0)
        .map(|target| u64::from(target.weight))
        .sum::<u64>();
    if total == 0 {
        return None;
    }
    let mut slot = cursor % total;
    for target in cluster
        .targets
        .iter()
        .filter(|target| target.enabled && target.weight > 0)
    {
        let weight = u64::from(target.weight);
        if slot < weight {
            return Some(target);
        }
        slot -= weight;
    }
    None
}

#[must_use]
pub fn rewrite_path(route: &RouteConfig, incoming_path: &str) -> String {
    if !route.strip_prefix || route.path_prefix == "/" {
        return incoming_path.to_owned();
    }
    let suffix = incoming_path
        .strip_prefix(&route.path_prefix)
        .unwrap_or(incoming_path);
    if suffix.is_empty() {
        "/".to_owned()
    } else if suffix.starts_with('/') {
        suffix.to_owned()
    } else {
        format!("/{suffix}")
    }
}

fn route_matches(route: &RouteConfig, request: RequestKey<'_>) -> bool {
    method_matches(route, request.method)
        && host_matches(route, request.host)
        && path_matches(&route.path_prefix, request.path)
}

fn method_matches(route: &RouteConfig, method: &str) -> bool {
    route.methods.iter().any(|candidate| candidate == method)
}

fn host_matches(route: &RouteConfig, host: Option<&str>) -> bool {
    if route.hosts.is_empty() {
        return true;
    }
    let Some(host) = host else {
        return false;
    };
    route
        .hosts
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(host))
}

fn path_matches(prefix: &str, path: &str) -> bool {
    if prefix == "/" {
        return path.starts_with('/');
    }
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IngressConfig;

    fn fixture() -> IngressConfig {
        IngressConfig::from_toml_str(
            r#"
schema_version = "ores.lambda.ingress-config/v1"
[controller]
bind = "127.0.0.1:8080"
request_timeout_ms = 1000
max_request_body_bytes = 1024
max_response_body_bytes = 2048
max_attempts = 2
trust_forwarded_headers = false
[[clusters]]
id = "c"
strategy = "weighted_round_robin"
[[clusters.targets]]
id = "a"
endpoint = "http://127.0.0.1:9001"
weight = 2
enabled = true
[[clusters.targets]]
id = "b"
endpoint = "http://127.0.0.1:9002"
weight = 1
enabled = true
[[routes]]
id = "specific"
hosts = ["api.example.test"]
path_prefix = "/v1/users"
methods = ["GET"]
cluster = "c"
priority = 10
strip_prefix = true
preserve_host = false
[[routes]]
id = "fallback"
hosts = []
path_prefix = "/"
methods = ["GET"]
cluster = "c"
priority = 1
strip_prefix = false
preserve_host = false
"#,
        )
        .unwrap()
    }

    #[test]
    fn longest_prefix_wins_after_priority() {
        let config = fixture();
        let matched = match_route(
            &config,
            RequestKey {
                method: "GET",
                host: Some("api.example.test"),
                path: "/v1/users/42",
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(matched.id, "specific");
        assert_eq!(rewrite_path(matched, "/v1/users/42"), "/42");
    }

    #[test]
    fn weighted_round_robin_is_deterministic() {
        let config = fixture();
        let cluster = config.cluster("c").unwrap();
        let ids = (0..6)
            .map(|cursor| select_target(cluster, cursor).unwrap().id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "a", "b", "a", "a", "b"]);
    }
}
