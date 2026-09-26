use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "scintilla.run/lambda/v1";
pub const INVOCATION_PROTOCOL: &str = "stdio-json-v1";
pub const CONTEXT_ABI: &str = "scintilla.run/context/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleKind {
    Lambda,
    Middleware,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleDescriptor {
    pub kind: ModuleKind,
    pub export_name: String,
    pub context_abi: String,
}

impl ModuleDescriptor {
    pub fn lambda(export_name: impl Into<String>) -> Self {
        Self {
            kind: ModuleKind::Lambda,
            export_name: export_name.into(),
            context_abi: CONTEXT_ABI.into(),
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if !valid_module_export(&self.export_name) {
            return Err(ValidationError::new(
                "exportName",
                "module export name is invalid",
            ));
        }
        if self.context_abi != CONTEXT_ABI {
            return Err(ValidationError::new(
                "contextAbi",
                "unsupported context ABI",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvocationContext {
    pub abi: String,
    pub invocation_id: String,
    pub timeout_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
}

pub trait LambdaHandler<Input, Output> {
    type Error;

    fn run(&self, payload: Input, ctx: &InvocationContext) -> Result<Output, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Nodejs,
    Bun,
    Deno,
    Rust,
    Erlang,
    Gleam,
    Golang,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerFormat {
    Docker,
    Oci,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperatingSystem {
    Linux,
    Darwin,
    Windows,
    Freebsd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    Amd64,
    Arm64,
    Armv7,
    Riscv64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Artifact {
    Container {
        format: ContainerFormat,
        image: String,
        digest: String,
        entrypoint: Vec<String>,
    },
    Executable {
        command: String,
        sha256: String,
        os: OperatingSystem,
        architecture: Architecture,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LambdaManifest {
    pub api_version: String,
    pub name: String,
    pub runtime: Runtime,
    pub protocol: String,
    pub handler: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    pub artifact: Artifact,
}

impl LambdaManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.api_version != API_VERSION {
            return Err(ValidationError::new(
                "apiVersion",
                "unsupported API version",
            ));
        }
        if !valid_name(&self.name) {
            return Err(ValidationError::new("name", "invalid lambda name"));
        }
        if self.protocol != INVOCATION_PROTOCOL {
            return Err(ValidationError::new(
                "protocol",
                "unsupported invocation protocol",
            ));
        }
        if !valid_bounded_string(&self.handler, 1, 64) {
            return Err(ValidationError::new(
                "handler",
                "handler must contain 1 to 64 characters",
            ));
        }
        if self
            .runtime_version
            .as_ref()
            .is_some_and(|version| !valid_bounded_string(version, 1, 64))
        {
            return Err(ValidationError::new(
                "runtimeVersion",
                "runtime version must contain 1 to 64 characters",
            ));
        }

        match &self.artifact {
            Artifact::Container {
                image,
                digest,
                entrypoint,
                ..
            } => {
                if !valid_image(image) {
                    return Err(ValidationError::new(
                        "artifact.image",
                        "container image must contain 1 to 512 bytes",
                    ));
                }
                if !digest.starts_with("sha256:") || !valid_sha256(&digest[7..]) {
                    return Err(ValidationError::new(
                        "artifact.digest",
                        "container image must be SHA-256 pinned",
                    ));
                }
                if entrypoint.is_empty() || entrypoint.len() > 64 {
                    return Err(ValidationError::new(
                        "artifact.entrypoint",
                        "entrypoint must contain 1 to 64 argv values",
                    ));
                }
                if !entrypoint.iter().all(|argument| valid_argument(argument)) {
                    return Err(ValidationError::new(
                        "artifact.entrypoint",
                        "entrypoint values must contain 1 to 1024 characters",
                    ));
                }
            }
            Artifact::Executable {
                command,
                sha256,
                args,
                ..
            } => {
                if !valid_command(command) {
                    return Err(ValidationError::new(
                        "artifact.command",
                        "command must be an absolute or ./ path without shell text",
                    ));
                }
                if !valid_sha256(sha256) {
                    return Err(ValidationError::new(
                        "artifact.sha256",
                        "executable checksum must be lowercase SHA-256",
                    ));
                }
                if args.len() > 64 {
                    return Err(ValidationError::new(
                        "artifact.args",
                        "args must contain at most 64 values",
                    ));
                }
                if !args.iter().all(|argument| valid_argument(argument)) {
                    return Err(ValidationError::new(
                        "artifact.args",
                        "argument values must contain 1 to 1024 characters",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: &'static str,
    pub message: &'static str,
}

impl ValidationError {
    const fn new(path: &'static str, message: &'static str) -> Self {
        Self { path, message }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for ValidationError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvocationRequest<T = serde_json::Value> {
    pub protocol: String,
    pub invocation_id: String,
    pub timeout_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    pub payload: T,
}

impl<T> InvocationRequest<T> {
    pub fn context(&self) -> InvocationContext {
        InvocationContext {
            abi: CONTEXT_ABI.into(),
            invocation_id: self.invocation_id.clone(),
            timeout_ms: self.timeout_ms,
            traceparent: self.traceparent.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvocationError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
pub enum InvocationResult<T = serde_json::Value> {
    Ok { payload: T },
    Error { error: InvocationError },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvocationResponse<T = serde_json::Value> {
    pub protocol: String,
    pub invocation_id: String,
    pub result: InvocationResult<T>,
}

fn valid_module_export(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(byte))
}

fn valid_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_image(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:/@-".contains(&byte))
        })
}

fn valid_argument(value: &str) -> bool {
    valid_bounded_string(value, 1, 1024)
}

fn valid_bounded_string(value: &str, minimum: usize, maximum: usize) -> bool {
    let length = value.chars().count();
    length >= minimum && length <= maximum
}

fn valid_command(value: &str) -> bool {
    (value.starts_with('/') || value.starts_with("./"))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._/+-".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn binary_manifest() -> LambdaManifest {
        LambdaManifest {
            api_version: API_VERSION.into(),
            name: "example-lambda".into(),
            runtime: Runtime::Binary,
            protocol: INVOCATION_PROTOCOL.into(),
            handler: "main".into(),
            runtime_version: None,
            artifact: Artifact::Executable {
                command: "./bin/lambda".into(),
                sha256: "a".repeat(64),
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                args: vec![],
            },
        }
    }

    #[test]
    fn invocation_context_and_descriptor_pin_the_public_abi() {
        let request = InvocationRequest {
            protocol: INVOCATION_PROTOCOL.into(),
            invocation_id: "rust-ctx".into(),
            timeout_ms: 900,
            traceparent: None,
            payload: 1u8,
        };
        let context = request.context();
        assert_eq!(context.abi, CONTEXT_ABI);
        assert_eq!(context.invocation_id, "rust-ctx");
        assert_eq!(context.timeout_ms, 900);
        let descriptor = ModuleDescriptor::lambda("run");
        assert_eq!(descriptor.context_abi, CONTEXT_ABI);
        assert_eq!(descriptor.validate(), Ok(()));

        let mut stale = descriptor.clone();
        stale.context_abi = "scintilla.run/context/v0".into();
        assert_eq!(stale.validate().unwrap_err().path, "contextAbi");
    }

    #[test]
    fn accepts_immutable_bare_executable() {
        assert_eq!(binary_manifest().validate(), Ok(()));
    }

    #[test]
    fn rejects_shell_command() {
        let mut manifest = binary_manifest();
        if let Artifact::Executable { command, .. } = &mut manifest.artifact {
            *command = "sh -c ./lambda".into();
        }
        assert_eq!(manifest.validate().unwrap_err().path, "artifact.command");
    }

    #[test]
    fn string_bounds_count_unicode_characters() {
        let mut manifest = binary_manifest();
        manifest.handler = "🚀".repeat(64);
        if let Artifact::Executable { args, .. } = &mut manifest.artifact {
            args.push("🚀".repeat(1024));
        }
        assert_eq!(manifest.validate(), Ok(()));

        manifest.handler.push('🚀');
        assert_eq!(manifest.validate().unwrap_err().path, "handler");
    }

    #[test]
    fn rejects_mixed_invocation_results() {
        let mixed = r#"{
            "protocol":"stdio-json-v1",
            "invocationId":"id-1",
            "result":{"status":"ok","payload":42,"error":{"code":"unexpected","message":"mixed","retryable":false}}
        }"#;
        assert!(serde_json::from_str::<InvocationResponse>(mixed).is_err());
    }

    #[test]
    fn classifies_the_shared_manifest_fixture_corpus() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/LambdaManifest");
        for (directory, expected) in [("valid", true), ("invalid", false)] {
            for entry in fs::read_dir(fixtures.join(directory)).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                let classified_valid =
                    serde_json::from_slice::<LambdaManifest>(&fs::read(&path).unwrap())
                        .is_ok_and(|manifest| manifest.validate().is_ok());
                assert_eq!(
                    classified_valid,
                    expected,
                    "unexpected fixture classification: {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn invocation_result_is_discriminated() {
        let response = InvocationResponse {
            protocol: INVOCATION_PROTOCOL.into(),
            invocation_id: "id-1".into(),
            result: InvocationResult::Ok { payload: 42 },
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["result"]["status"], "ok");
    }
}
