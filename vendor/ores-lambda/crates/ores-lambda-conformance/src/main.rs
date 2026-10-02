mod lifecycle;

use anyhow::{bail, Context};
use ores_lambda_core::{
    match_route, ConformanceReceipt, IngressConfig, LambdaRepositoryDescriptor, RequestKey,
};
use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
struct RouteVectorFile {
    config: String,
    cases: Vec<RouteVectorCase>,
}

#[derive(Debug, Deserialize)]
struct RouteVectorCase {
    method: String,
    host: String,
    path: String,
    expected_route: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [command, path] if command == "ingress" => check_ingress(Path::new(path)),
        [command, path] if command == "repository" => check_repository(Path::new(path)),
        [command, root] if command == "self-test" => self_test(Path::new(root)),
        [command] if command == "self-test" => self_test(Path::new(".")),
        [command, root] if command == "lifecycle" => lifecycle::check(Path::new(root)),
        [command] if command == "lifecycle" => lifecycle::check(Path::new(".")),
        [command, root] if command == "package" => lifecycle::check_package(Path::new(root)),
        [command] if command == "package" => lifecycle::check_package(Path::new(".")),
        _ => bail!(
            "usage: ores-lambda-conformance ingress <.ores-lambda.toml> | repository <lambda-repository.json> | self-test [repo-root] | lifecycle [repo-root] | package [package-root]"
        ),
    }
}

fn check_ingress(path: &Path) -> anyhow::Result<()> {
    let input = fs::read_to_string(path)
        .with_context(|| format!("read ingress config {}", path.display()))?;
    let config: IngressConfig = toml::from_str(&input).context("parse ingress config")?;
    let receipt = ConformanceReceipt::ingress(&config)?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    if receipt.passed {
        Ok(())
    } else {
        bail!("ingress config failed conformance")
    }
}

fn check_repository(path: &Path) -> anyhow::Result<()> {
    let input = fs::read_to_string(path)
        .with_context(|| format!("read lambda repository descriptor {}", path.display()))?;
    let descriptor: LambdaRepositoryDescriptor =
        serde_json::from_str(&input).context("parse lambda repository descriptor")?;
    let receipt = ConformanceReceipt::repository(&descriptor)?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    if receipt.passed {
        Ok(())
    } else {
        bail!("lambda repository descriptor failed conformance")
    }
}

fn self_test(root: &Path) -> anyhow::Result<()> {
    let valid_ingress_dir = root.join("conformance/fixtures/ingress/valid");
    let invalid_ingress_dir = root.join("conformance/fixtures/ingress/invalid");
    let valid_repository_dir = root.join("conformance/fixtures/repository/valid");
    let invalid_repository_dir = root.join("conformance/fixtures/repository/invalid");

    let valid_ingress_files = fixture_files(&valid_ingress_dir, "toml")?;
    let invalid_ingress_files = fixture_files(&invalid_ingress_dir, "toml")?;
    let valid_repository_files = fixture_files(&valid_repository_dir, "json")?;
    let invalid_repository_files = fixture_files(&invalid_repository_dir, "json")?;

    for path in &valid_ingress_files {
        let config = load_ingress_unchecked(path)?;
        let violations = config.validate();
        if !violations.is_empty() {
            bail!(
                "valid ingress fixture {} was rejected with {:?}",
                path.display(),
                violations
            )
        }
    }
    for path in &invalid_ingress_files {
        let config = load_ingress_unchecked(path)?;
        if config.validate().is_empty() {
            bail!("invalid ingress fixture {} was admitted", path.display())
        }
    }

    assert_ingress_violation(
        &invalid_ingress_dir.join("ambiguous-route.toml"),
        "ambiguous_route",
    )?;
    assert_ingress_violation(
        &invalid_ingress_dir.join("unproven-forwarded-trust.toml"),
        "forwarded_headers_trust",
    )?;

    for path in &valid_repository_files {
        let descriptor = load_repository_unchecked(path)?;
        let violations = descriptor.validate();
        if !violations.is_empty() {
            bail!(
                "valid repository fixture {} was rejected with {:?}",
                path.display(),
                violations
            )
        }
    }
    for path in &invalid_repository_files {
        let descriptor = load_repository_unchecked(path)?;
        if descriptor.validate().is_empty() {
            bail!("invalid repository fixture {} was admitted", path.display())
        }
    }

    let portable_repository = valid_repository_dir.join("portable.json");
    let portable_repo = load_repository_unchecked(&portable_repository)?;
    if portable_repo.providers.is_empty() || portable_repo.portable_contract.is_none() {
        bail!("portable repository fixture did not retain provider metadata")
    }

    let artifact_policy = invalid_repository_dir.join("artifact-policy.json");
    let invalid_codes = load_repository_unchecked(&artifact_policy)?
        .validate()
        .into_iter()
        .map(|violation| violation.code)
        .collect::<Vec<_>>();
    for expected in [
        "artifact_compiler",
        "artifact_source",
        "artifact_credentials",
        "artifact_provenance",
        "mutable_shared_core",
    ] {
        if !invalid_codes.iter().any(|code| code == expected) {
            bail!("invalid repository fixture did not produce {expected}")
        }
    }

    let vector_count = check_route_vectors(root)?;
    println!(
        "ores-lambda conformance self-test passed: ingress_valid={} ingress_invalid={} repository_valid={} repository_invalid={} vector_files={vector_count}",
        valid_ingress_files.len(),
        invalid_ingress_files.len(),
        valid_repository_files.len(),
        invalid_repository_files.len(),
    );
    Ok(())
}

fn assert_ingress_violation(path: &Path, expected: &str) -> anyhow::Result<()> {
    let config = load_ingress_unchecked(path)?;
    if !config
        .validate()
        .iter()
        .any(|violation| violation.code == expected)
    {
        bail!(
            "invalid ingress fixture {} did not produce {expected}",
            path.display()
        )
    }
    Ok(())
}

fn fixture_files(directory: &Path, extension: &str) -> anyhow::Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(directory)
        .with_context(|| format!("fixture directory missing: {}", directory.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "fixture directory must be a real directory: {}",
            directory.display()
        )
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(directory)
        .with_context(|| format!("read fixture directory {}", directory.display()))?
    {
        let entry = entry.context("read fixture entry")?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect fixture {}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!(
                "fixture must be a regular non-symlink file: {}",
                path.display()
            )
        }
        if path.extension().and_then(|value| value.to_str()) != Some(extension) {
            bail!(
                "unexpected fixture extension in {}: expected .{extension}",
                path.display()
            )
        }
        files.push(path);
    }
    files.sort();
    if files.is_empty() {
        bail!("fixture directory is empty: {}", directory.display())
    }
    Ok(files)
}

fn check_route_vectors(root: &Path) -> anyhow::Result<usize> {
    let vector_dir = root.join("conformance/vectors");
    let vector_files = fixture_files(&vector_dir, "json")?;
    for path in &vector_files {
        let input = fs::read_to_string(path)
            .with_context(|| format!("read route vectors {}", path.display()))?;
        let vectors: RouteVectorFile = serde_json::from_str(&input)
            .with_context(|| format!("parse route vectors {}", path.display()))?;
        let configured_path = root.join(PathBuf::from(&vectors.config));
        let config = load_ingress_unchecked(&configured_path).with_context(|| {
            format!(
                "route vector {} references missing or invalid config {}",
                path.display(),
                configured_path.display()
            )
        })?;

        for case in vectors.cases {
            let matched = match_route(
                &config,
                RequestKey {
                    method: &case.method,
                    host: Some(&case.host),
                    path: &case.path,
                },
            )
            .with_context(|| {
                format!(
                    "route vector {} produced ambiguous best match for {} {} {}",
                    path.display(),
                    case.method,
                    case.host,
                    case.path
                )
            })?;
            let actual = matched.map(|route| route.id.as_str());
            if actual != case.expected_route.as_deref() {
                bail!(
                    "route vector mismatch in {} for {} {} {}: expected {:?}, got {:?}",
                    path.display(),
                    case.method,
                    case.host,
                    case.path,
                    case.expected_route,
                    actual
                )
            }
        }
    }
    Ok(vector_files.len())
}

fn load_ingress_unchecked(path: &Path) -> anyhow::Result<IngressConfig> {
    let input = fs::read_to_string(path)
        .with_context(|| format!("read ingress fixture {}", path.display()))?;
    toml::from_str(&input).context("parse ingress fixture")
}

fn load_repository_unchecked(path: &Path) -> anyhow::Result<LambdaRepositoryDescriptor> {
    let input = fs::read_to_string(path)
        .with_context(|| format!("read repository fixture {}", path.display()))?;
    serde_json::from_str(&input).context("parse repository fixture")
}
