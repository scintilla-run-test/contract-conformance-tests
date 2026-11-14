use crate::Violation;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};
use thiserror::Error;

pub const LAMBDA_REPOSITORY_SCHEMA: &str = "ores.lambda-repository.v1";
const IMMUTABLE_RESOLUTION: &str = "pin_exact_reviewed_package_or_commit_before_use";
const JSON_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LambdaRepositoryDescriptor {
    pub schema_version: String,
    pub organization: String,
    pub repository: String,
    pub runtime: String,
    pub architectures: Vec<String>,
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub entrypoints: BTreeMap<String, String>,
    pub portable_contract: Option<PortableContract>,
    pub artifact_contract: LambdaArtifactContract,
    pub shared_core_dependencies: SharedCoreDependencies,
    pub ownership: LambdaOwnership,
    #[serde(default)]
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LambdaArtifactContract {
    pub zip_root_executable: String,
    pub compiler_in_runtime_artifact_allowed: bool,
    pub source_in_runtime_artifact_allowed: bool,
    pub credentials_in_artifact_allowed: bool,
    pub exact_source_commit_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharedCoreDependencies {
    pub domain_candidate: String,
    pub orm_candidate: String,
    pub resolution: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LambdaOwnership {
    pub cross_server_functions: String,
    pub route_local_functions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortableContract {
    pub schema_version: String,
    pub maximum_command_bytes: u64,
    pub maximum_invocation_bytes: u64,
    pub unknown_fields_allowed: bool,
    pub provider_owns_http_listener: bool,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("read lambda repository descriptor {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parse lambda repository descriptor: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("lambda repository descriptor failed validation: {0:?}")]
    Invalid(Vec<Violation>),
}

impl LambdaRepositoryDescriptor {
    pub fn from_json_str(input: &str) -> Result<Self, RepositoryError> {
        let descriptor: Self = serde_json::from_str(input)?;
        let violations = descriptor.validate();
        if violations.is_empty() {
            Ok(descriptor)
        } else {
            Err(RepositoryError::Invalid(violations))
        }
    }

    pub fn load(path: &Path) -> Result<Self, RepositoryError> {
        let input = fs::read_to_string(path).map_err(|source| RepositoryError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_json_str(&input)
    }

    #[must_use]
    pub fn validate(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        if self.schema_version != LAMBDA_REPOSITORY_SCHEMA {
            violations.push(violation(
                "schema_version",
                "schemaVersion",
                format!("expected {LAMBDA_REPOSITORY_SCHEMA:?}"),
            ));
        }
        for (path, value) in [
            ("organization", self.organization.as_str()),
            ("repository", self.repository.as_str()),
            ("runtime", self.runtime.as_str()),
        ] {
            if value.trim().is_empty() {
                violations.push(violation("required", path, "value must not be empty"));
            }
        }
        if self.architectures.is_empty() {
            violations.push(violation(
                "architectures",
                "architectures",
                "at least one architecture is required",
            ));
        }
        reject_duplicates("architectures", &self.architectures, &mut violations);
        reject_duplicates("providers", &self.providers, &mut violations);
        for (name, entrypoint) in &self.entrypoints {
            if name.trim().is_empty() || entrypoint.trim().is_empty() {
                violations.push(violation(
                    "entrypoint",
                    format!("entrypoints.{name}"),
                    "entrypoint names and values must not be empty",
                ));
            }
        }
        if let Some(portable) = &self.portable_contract {
            if portable.schema_version.trim().is_empty() {
                violations.push(violation(
                    "portable_schema",
                    "portableContract.schemaVersion",
                    "portable contract schemaVersion must not be empty",
                ));
            }
            if portable.maximum_command_bytes == 0
                || portable.maximum_invocation_bytes < portable.maximum_command_bytes
            {
                violations.push(violation(
                    "portable_bounds",
                    "portableContract.maximumInvocationBytes",
                    "maximumInvocationBytes must be >= non-zero maximumCommandBytes",
                ));
            }
            if portable.maximum_command_bytes > JSON_MAX_SAFE_INTEGER
                || portable.maximum_invocation_bytes > JSON_MAX_SAFE_INTEGER
            {
                violations.push(violation(
                    "portable_json_safe_integer",
                    "portableContract",
                    "portable byte bounds must fit JavaScript's exact JSON integer range",
                ));
            }
            if portable.unknown_fields_allowed {
                violations.push(violation(
                    "portable_unknown_fields",
                    "portableContract.unknownFieldsAllowed",
                    "portable command contracts must reject unknown fields",
                ));
            }
        }
        let artifact = &self.artifact_contract;
        if artifact.compiler_in_runtime_artifact_allowed {
            violations.push(violation(
                "artifact_compiler",
                "artifactContract.compilerInRuntimeArtifactAllowed",
                "runtime artifacts must not contain a compiler",
            ));
        }
        if artifact.source_in_runtime_artifact_allowed {
            violations.push(violation(
                "artifact_source",
                "artifactContract.sourceInRuntimeArtifactAllowed",
                "runtime artifacts must not contain source trees",
            ));
        }
        if artifact.credentials_in_artifact_allowed {
            violations.push(violation(
                "artifact_credentials",
                "artifactContract.credentialsInArtifactAllowed",
                "runtime artifacts must not contain credentials",
            ));
        }
        if !artifact.exact_source_commit_required {
            violations.push(violation(
                "artifact_provenance",
                "artifactContract.exactSourceCommitRequired",
                "artifacts must bind the exact source commit",
            ));
        }
        if self.runtime.starts_with("provided.") && artifact.zip_root_executable != "bootstrap" {
            violations.push(violation(
                "aws_bootstrap",
                "artifactContract.zipRootExecutable",
                "AWS OS-only runtime ZIPs require root executable 'bootstrap'",
            ));
        }
        if self.shared_core_dependencies.resolution != IMMUTABLE_RESOLUTION {
            violations.push(violation(
                "mutable_shared_core",
                "sharedCoreDependencies.resolution",
                format!("expected {IMMUTABLE_RESOLUTION:?}"),
            ));
        }
        if self.ownership.cross_server_functions != "this_repository" {
            violations.push(violation(
                "ownership",
                "ownership.crossServerFunctions",
                "cross-server functions must be owned by this repository",
            ));
        }
        if self.ownership.route_local_functions != "owning_server_src_lambdas_allowed" {
            violations.push(violation(
                "ownership",
                "ownership.routeLocalFunctions",
                "route-local functions must retain the owning-server exception",
            ));
        }
        violations.sort();
        violations.dedup();
        violations
    }
}

fn reject_duplicates(path: &str, values: &[String], violations: &mut Vec<Violation>) {
    let mut sorted = values.to_vec();
    sorted.sort();
    let before = sorted.len();
    sorted.dedup();
    if sorted.len() != before {
        violations.push(violation(
            path,
            path,
            format!("{path} values must be unique"),
        ));
    }
}

fn violation(code: &str, path: impl Into<String>, message: impl Into<String>) -> Violation {
    Violation {
        code: code.to_owned(),
        path: path.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_v1_wire_names_deserialize() {
        let descriptor = LambdaRepositoryDescriptor::from_json_str(
            r#"{
              "schemaVersion":"ores.lambda-repository.v1",
              "organization":"demo",
              "repository":"demo-lambdas",
              "runtime":"provided.al2023",
              "architectures":["x86_64","arm64"],
              "artifactContract":{
                "zipRootExecutable":"bootstrap",
                "compilerInRuntimeArtifactAllowed":false,
                "sourceInRuntimeArtifactAllowed":false,
                "credentialsInArtifactAllowed":false,
                "exactSourceCommitRequired":true
              },
              "sharedCoreDependencies":{
                "domainCandidate":"demo/demo-lib-core",
                "ormCandidate":"demo/demo-orm-core",
                "resolution":"pin_exact_reviewed_package_or_commit_before_use"
              },
              "ownership":{
                "crossServerFunctions":"this_repository",
                "routeLocalFunctions":"owning_server_src_lambdas_allowed"
              }
            }"#,
        )
        .unwrap();
        assert_eq!(descriptor.organization, "demo");
    }

    #[test]
    fn portable_bounds_reject_json_unsafe_integers() {
        let input = format!(
            r#"{{
              "schemaVersion":"ores.lambda-repository.v1",
              "organization":"demo",
              "repository":"demo-lambdas",
              "runtime":"provided.al2023",
              "architectures":["x86_64"],
              "portableContract":{{
                "schemaVersion":"demo.worker-command.v1",
                "maximumCommandBytes":{},
                "maximumInvocationBytes":{},
                "unknownFieldsAllowed":false,
                "providerOwnsHttpListener":true
              }},
              "artifactContract":{{
                "zipRootExecutable":"bootstrap",
                "compilerInRuntimeArtifactAllowed":false,
                "sourceInRuntimeArtifactAllowed":false,
                "credentialsInArtifactAllowed":false,
                "exactSourceCommitRequired":true
              }},
              "sharedCoreDependencies":{{
                "domainCandidate":"demo/demo-lib-core",
                "ormCandidate":"demo/demo-orm-core",
                "resolution":"pin_exact_reviewed_package_or_commit_before_use"
              }},
              "ownership":{{
                "crossServerFunctions":"this_repository",
                "routeLocalFunctions":"owning_server_src_lambdas_allowed"
              }}
            }}"#,
            JSON_MAX_SAFE_INTEGER + 1,
            JSON_MAX_SAFE_INTEGER + 1,
        );
        let error = LambdaRepositoryDescriptor::from_json_str(&input).unwrap_err();
        assert!(matches!(
            error,
            RepositoryError::Invalid(ref violations)
                if violations.iter().any(|violation| violation.code == "portable_json_safe_integer")
        ));
    }

    #[test]
    fn portable_provider_extensions_remain_v1_compatible() {
        let descriptor = LambdaRepositoryDescriptor::from_json_str(
            r#"{
              "schemaVersion":"ores.lambda-repository.v1",
              "organization":"demo",
              "repository":"demo-lambdas",
              "runtime":"provided.al2023",
              "architectures":["x86_64","arm64"],
              "providers":["aws-lambda","google-cloud-functions","local"],
              "entrypoints":{"awsWorker":"worker-aws","portableStdioWorker":"worker-portable"},
              "portableContract":{
                "schemaVersion":"demo.worker-command.v1",
                "maximumCommandBytes":262144,
                "maximumInvocationBytes":263168,
                "unknownFieldsAllowed":false,
                "providerOwnsHttpListener":true
              },
              "artifactContract":{
                "zipRootExecutable":"bootstrap",
                "compilerInRuntimeArtifactAllowed":false,
                "sourceInRuntimeArtifactAllowed":false,
                "credentialsInArtifactAllowed":false,
                "exactSourceCommitRequired":true
              },
              "sharedCoreDependencies":{
                "domainCandidate":"demo/demo-lib-core",
                "ormCandidate":"demo/demo-orm-core",
                "resolution":"pin_exact_reviewed_package_or_commit_before_use"
              },
              "ownership":{
                "crossServerFunctions":"this_repository",
                "routeLocalFunctions":"owning_server_src_lambdas_allowed"
              }
            }"#,
        )
        .unwrap();
        assert_eq!(descriptor.providers.len(), 3);
        assert!(descriptor.portable_contract.is_some());
    }
}
