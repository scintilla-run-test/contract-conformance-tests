use anyhow::{bail, Context};
use ores_lambda_core::{LambdaManifest, LambdaSourceKind};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Component, Path, PathBuf},
};

const DEFAULT_MANIFEST: &str = ".ores-lambda.toml";
const DEFAULT_INGRESS: &str = ".ores/generated/lambda-ingress.toml";
const MIDDLEWARE_PACKAGE: &str = "ores-middleware";
const MIDDLEWARE_ZPKG_PACKAGE: &str = "oresoftware/ores-middleware";
const MAX_ORES_COMPOSE_REPLICAS: u16 = 32;

fn main() -> anyhow::Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [command] if command == "check" => check(Path::new(DEFAULT_MANIFEST)),
        [command, manifest] if command == "check" => check(Path::new(manifest)),
        [command] if command == "check-repo" => check_repo(Path::new(".")),
        [command, root] if command == "check-repo" => check_repo(Path::new(root)),
        [command] if command == "ingress" => ingress(Path::new(DEFAULT_MANIFEST)),
        [command, manifest] if command == "ingress" => ingress(Path::new(manifest)),
        [command] if command == "compose" => compose(Path::new(DEFAULT_MANIFEST), DEFAULT_INGRESS),
        [command, manifest] if command == "compose" => {
            compose(Path::new(manifest), DEFAULT_INGRESS)
        }
        [command, manifest, ingress_path] if command == "compose" => {
            compose(Path::new(manifest), ingress_path)
        }
        [command] if command == "materialize" => {
            materialize(Path::new(DEFAULT_MANIFEST), Path::new(DEFAULT_INGRESS))
        }
        [command, manifest] if command == "materialize" => {
            materialize(Path::new(manifest), Path::new(DEFAULT_INGRESS))
        }
        [command, manifest, output] if command == "materialize" => {
            materialize(Path::new(manifest), Path::new(output))
        }
        _ => bail!(
            "usage: ores-lambda check [manifest] | check-repo [repo-root] | ingress [manifest] | compose [manifest] [ingress-config-path] | materialize [manifest] [output]"
        ),
    }
}

fn load(path: &Path) -> anyhow::Result<LambdaManifest> {
    LambdaManifest::load(path).with_context(|| format!("admit lambda manifest {}", path.display()))
}

fn check(path: &Path) -> anyhow::Result<()> {
    let manifest = load(path)?;
    let ingress = manifest.ingress_config().map_err(|violations| {
        anyhow::anyhow!("derived ingress failed validation: {violations:?}")
    })?;
    println!(
        "lambda manifest admitted: repository={}/{} functions={} local_routes={} middleware=required/invocation/fail_closed",
        manifest.repository.organization,
        manifest.repository.repository,
        manifest.functions.len(),
        ingress.routes.len(),
    );
    Ok(())
}

fn check_repo(root: &Path) -> anyhow::Result<()> {
    ensure_real_dir(root, "repository root")?;
    let manifest_path = root.join(DEFAULT_MANIFEST);
    ensure_regular_file(&manifest_path, DEFAULT_MANIFEST)?;
    let manifest = load(&manifest_path)?;
    manifest.ingress_config().map_err(|violations| {
        anyhow::anyhow!("derived ingress failed validation: {violations:?}")
    })?;

    for boundary in ["contracts", "conformance"] {
        ensure_real_dir(&root.join(boundary), boundary)?;
    }

    let middleware_path = admitted_relative_path(root, &manifest.middleware.config_path)?;
    ensure_regular_file(&middleware_path, "middleware config")?;
    let stack_path = middleware_stack_path(root, &middleware_path)?;
    ensure_regular_file(&stack_path, "middleware stack config")?;

    let cargo_path = root.join("Cargo.toml");
    ensure_regular_file(&cargo_path, "Cargo.toml")?;
    let cargo = parse_toml_file(&cargo_path)?;
    if !table_declares_package(&cargo, "dependencies", MIDDLEWARE_PACKAGE) {
        bail!("Cargo.toml must declare the canonical {MIDDLEWARE_PACKAGE} dependency");
    }
    let cargo_binaries = cargo_binary_names(root, &cargo)?;
    ensure_regular_file(&root.join("Cargo.lock"), "Cargo.lock")?;

    let zpkg_path = root.join(".zpkg.toml");
    ensure_regular_file(&zpkg_path, ".zpkg.toml")?;
    let zpkg = parse_toml_file(&zpkg_path)?;
    if !table_has_key(&zpkg, "dependencies", MIDDLEWARE_ZPKG_PACKAGE) {
        bail!(".zpkg.toml must declare the canonical {MIDDLEWARE_ZPKG_PACKAGE} dependency");
    }
    for target in ["contracts", "conformance"] {
        if !table_has_key(&zpkg, "targets", target) {
            bail!(".zpkg.toml must publish [targets.{target}]");
        }
    }

    for function in &manifest.functions {
        if function.source.kind == LambdaSourceKind::Custom
            && function.source.repository == manifest.repository.repository
        {
            let source = admitted_relative_path(root, &function.source.path)?;
            ensure_regular_file(
                &source,
                &format!("custom Lambda source for {}", function.id),
            )?;
            if !cargo_binaries.contains(&function.entrypoint) {
                bail!(
                    "custom Lambda {:?} entrypoint {:?} is not a declared or auto-discovered Cargo binary",
                    function.id,
                    function.entrypoint
                );
            }
        }
        // `local.working_dir` belongs to the ores-compose workspace authority.
        // The manifest validator proves that it is a safe relative path; this
        // repo-local admission command must not require a sibling monorepo path
        // to exist underneath the standalone *-lambdas checkout.
    }

    println!(
        "lambda repository admitted: repository={}/{} functions={} cargo_bins={} middleware={} stack={} contracts=present conformance=present",
        manifest.repository.organization,
        manifest.repository.repository,
        manifest.functions.len(),
        cargo_binaries.len(),
        relative_display(root, &middleware_path),
        relative_display(root, &stack_path),
    );
    Ok(())
}

fn ingress(path: &Path) -> anyhow::Result<()> {
    let manifest = load(path)?;
    let config = manifest.ingress_config().map_err(|violations| {
        anyhow::anyhow!("derived ingress failed validation: {violations:?}")
    })?;
    print!(
        "{}",
        toml::to_string_pretty(&config).context("serialize derived ingress config")?
    );
    Ok(())
}

fn compose(path: &Path, ingress_config_path: &str) -> anyhow::Result<()> {
    let manifest = load(path)?;
    manifest.ingress_config().map_err(|violations| {
        anyhow::anyhow!("derived ingress failed validation: {violations:?}")
    })?;
    for function in &manifest.functions {
        if let Some(local) = &function.local {
            if local.replicas > MAX_ORES_COMPOSE_REPLICAS {
                bail!(
                    "function {:?} requests {} local replicas but ores-compose currently admits at most {}",
                    function.id,
                    local.replicas,
                    MAX_ORES_COMPOSE_REPLICAS
                );
            }
        }
    }
    print!(
        "{}",
        manifest.compose_services_fragment(ingress_config_path)
    );
    Ok(())
}

fn materialize(manifest_path: &Path, output_path: &Path) -> anyhow::Result<()> {
    let manifest = load(manifest_path)?;
    let config = manifest.ingress_config().map_err(|violations| {
        anyhow::anyhow!("derived ingress failed validation: {violations:?}")
    })?;
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let temporary = temp_sibling(output_path);
    fs::write(&temporary, toml::to_string_pretty(&config)?)
        .with_context(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, output_path)
        .with_context(|| format!("atomically publish {}", output_path.display()))?;
    println!("materialized {}", output_path.display());
    Ok(())
}

fn middleware_stack_path(root: &Path, middleware_path: &Path) -> anyhow::Result<PathBuf> {
    let middleware = parse_toml_file(middleware_path)?;
    let default_target = middleware
        .get("default_target")
        .and_then(toml::Value::as_str)
        .context(".ores-mw.toml must declare default_target")?;
    let targets = middleware
        .get("targets")
        .and_then(toml::Value::as_array)
        .context(".ores-mw.toml must declare [[targets]]")?;
    let target = targets
        .iter()
        .filter_map(toml::Value::as_table)
        .find(|target| {
            target
                .get("name")
                .and_then(toml::Value::as_str)
                .is_some_and(|name| name == default_target)
        })
        .with_context(|| {
            format!(".ores-mw.toml default_target {default_target:?} is not declared")
        })?;
    let stack_config = target
        .get("stack_config")
        .and_then(toml::Value::as_str)
        .context("default middleware target must declare stack_config")?;
    admitted_relative_path(root, stack_config)
}

fn parse_toml_file(path: &Path) -> anyhow::Result<toml::Value> {
    let input = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&input).with_context(|| format!("parse {}", path.display()))
}

fn table_has_key(document: &toml::Value, table: &str, key: &str) -> bool {
    document
        .get(table)
        .and_then(toml::Value::as_table)
        .is_some_and(|values| values.contains_key(key))
}

fn table_declares_package(document: &toml::Value, table: &str, package: &str) -> bool {
    let Some(values) = document.get(table).and_then(toml::Value::as_table) else {
        return false;
    };
    values.iter().any(|(key, value)| {
        key == package
            || value
                .as_table()
                .and_then(|dependency| dependency.get("package"))
                .and_then(toml::Value::as_str)
                .is_some_and(|declared| declared == package)
    })
}

fn cargo_binary_names(root: &Path, document: &toml::Value) -> anyhow::Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    let autobins = cargo_autobins(document)?;

    if let Some(binaries) = document.get("bin").and_then(toml::Value::as_array) {
        for (index, binary) in binaries.iter().enumerate() {
            let table = binary
                .as_table()
                .with_context(|| format!("Cargo [[bin]] entry {index} must be a table"))?;
            let name = table
                .get("name")
                .and_then(toml::Value::as_str)
                .with_context(|| format!("Cargo [[bin]] entry {index} must declare name"))?;
            if name.trim().is_empty() {
                bail!("Cargo [[bin]] entry {index} has an empty name");
            }

            match table.get("path") {
                Some(path) => {
                    let path = path.as_str().with_context(|| {
                        format!("Cargo [[bin]] entry {index} path must be a string")
                    })?;
                    let binary_source = admitted_relative_path(root, path)?;
                    ensure_regular_file(
                        &binary_source,
                        &format!("Cargo [[bin]] source for {name}"),
                    )?;
                }
                None => {
                    inferred_binary_source(root, document, name).with_context(|| {
                        format!("resolve inferred Cargo [[bin]] source for {name}")
                    })?;
                }
            }
            names.insert(name.to_owned());
        }
    }

    if autobins {
        let default_main = root.join("src/main.rs");
        if regular_non_symlink_file(&default_main)? {
            if let Some(package_name) = document
                .get("package")
                .and_then(toml::Value::as_table)
                .and_then(|package| package.get("name"))
                .and_then(toml::Value::as_str)
            {
                names.insert(package_name.to_owned());
            }
        }

        let bin_dir = root.join("src/bin");
        match fs::symlink_metadata(&bin_dir) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    bail!(
                        "src/bin must be a real non-symlink directory: {}",
                        bin_dir.display()
                    );
                }
                for entry in fs::read_dir(&bin_dir)
                    .with_context(|| format!("read Cargo binary directory {}", bin_dir.display()))?
                {
                    let entry =
                        entry.with_context(|| format!("read entry under {}", bin_dir.display()))?;
                    let path = entry.path();
                    let metadata = fs::symlink_metadata(&path).with_context(|| {
                        format!("inspect Cargo binary candidate {}", path.display())
                    })?;
                    if metadata.file_type().is_symlink() {
                        bail!(
                            "Cargo binary candidate must not be a symlink: {}",
                            path.display()
                        );
                    }
                    if metadata.is_file()
                        && path.extension().is_some_and(|extension| extension == "rs")
                    {
                        if let Some(name) = path.file_stem().and_then(|value| value.to_str()) {
                            names.insert(name.to_owned());
                        }
                    } else if metadata.is_dir() {
                        let main = path.join("main.rs");
                        if regular_non_symlink_file(&main)? {
                            if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                                names.insert(name.to_owned());
                            }
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect Cargo binary directory {}", bin_dir.display())
                });
            }
        }
    }

    Ok(names)
}

fn cargo_autobins(document: &toml::Value) -> anyhow::Result<bool> {
    let value = document
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("autobins"));
    match value {
        None => Ok(true),
        Some(value) => value
            .as_bool()
            .context("Cargo [package].autobins must be a boolean"),
    }
}

fn inferred_binary_source(
    root: &Path,
    document: &toml::Value,
    name: &str,
) -> anyhow::Result<PathBuf> {
    let mut candidates = Vec::new();

    let package_name = document
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str);
    if package_name == Some(name) {
        push_inferred_binary_candidate(root, &root.join("src/main.rs"), &mut candidates)?;
    }
    push_inferred_binary_candidate(
        root,
        &root.join("src/bin").join(format!("{name}.rs")),
        &mut candidates,
    )?;
    push_inferred_binary_candidate(
        root,
        &root.join("src/bin").join(name).join("main.rs"),
        &mut candidates,
    )?;

    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [path] => Ok(path.clone()),
        [] => bail!(
            "Cargo [[bin]] {name:?} has no inferred source at src/main.rs, src/bin/{name}.rs, or src/bin/{name}/main.rs"
        ),
        _ => bail!(
            "Cargo [[bin]] {name:?} has ambiguous inferred sources: {}",
            candidates
                .iter()
                .map(|path| relative_display(root, path))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn push_inferred_binary_candidate(
    root: &Path,
    path: &Path,
    candidates: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "Cargo inferred [[bin]] source must be a regular non-symlink file: {}",
                    relative_display(root, path)
                );
            }
            candidates.push(path.to_path_buf());
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}

fn regular_non_symlink_file(path: &Path) -> anyhow::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(!metadata.file_type().is_symlink() && metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}

fn admitted_relative_path(root: &Path, value: &str) -> anyhow::Result<PathBuf> {
    let relative = Path::new(value);
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        bail!("repository path must be a non-empty relative path: {value:?}");
    }
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("repository path must not traverse outside the root: {value:?}");
    }
    Ok(root.join(relative))
}

fn ensure_regular_file(path: &Path, label: &str) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("required {label} is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} must be a regular non-symlink file: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_real_dir(path: &Path, label: &str) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("required {label} is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "{label} must be a real non-symlink directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn temp_sibling(output: &Path) -> PathBuf {
    let mut value = output.as_os_str().to_os_string();
    value.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!(
            "ores-lambda-entrypoint-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/bin")).unwrap();
        root
    }

    #[test]
    fn cargo_binary_names_include_explicit_and_auto_discovered_bins() {
        let root = temp_root("discovery");
        fs::write(root.join("src/bin/worker_http.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(root.join("src/bin/health")).unwrap();
        fs::write(root.join("src/bin/health/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("explicit.rs"), "fn main() {}\n").unwrap();
        let cargo: toml::Value = toml::from_str(
            r#"
            [package]
            name = "implicit-main"

            [[bin]]
            name = "explicit-worker"
            path = "explicit.rs"
            "#,
        )
        .unwrap();

        let names = cargo_binary_names(&root, &cargo).unwrap();
        assert!(names.contains("explicit-worker"));
        assert!(names.contains("worker_http"));
        assert!(names.contains("health"));
        assert!(!names.contains("implicit-main"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cargo_binary_names_include_package_binary_when_src_main_exists() {
        let root = temp_root("main");
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        let cargo: toml::Value = toml::from_str(
            r#"
            [package]
            name = "repo-worker"
            "#,
        )
        .unwrap();

        let names = cargo_binary_names(&root, &cargo).unwrap();
        assert!(names.contains("repo-worker"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cargo_binary_names_honor_autobins_false() {
        let root = temp_root("autobins-false");
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("src/bin/worker.rs"), "fn main() {}\n").unwrap();
        let cargo: toml::Value = toml::from_str(
            r#"
            [package]
            name = "repo-worker"
            autobins = false
            "#,
        )
        .unwrap();

        let names = cargo_binary_names(&root, &cargo).unwrap();
        assert!(names.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_bin_without_path_uses_inferred_source_when_autobins_false() {
        let root = temp_root("explicit-inferred");
        fs::write(root.join("src/bin/worker.rs"), "fn main() {}\n").unwrap();
        let cargo: toml::Value = toml::from_str(
            r#"
            [package]
            name = "repo-worker"
            autobins = false

            [[bin]]
            name = "worker"
            "#,
        )
        .unwrap();

        let names = cargo_binary_names(&root, &cargo).unwrap();
        assert_eq!(names, BTreeSet::from(["worker".to_owned()]));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_bin_without_path_rejects_missing_inferred_source() {
        let root = temp_root("missing-inferred");
        let cargo: toml::Value = toml::from_str(
            r#"
            [package]
            name = "repo-worker"
            autobins = false

            [[bin]]
            name = "missing"
            "#,
        )
        .unwrap();

        let error = cargo_binary_names(&root, &cargo).unwrap_err();
        assert!(format!("{error:#}").contains("no inferred source"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn explicit_bin_without_path_rejects_symlinked_inferred_source() {
        use std::os::unix::fs::symlink;

        let root = temp_root("symlink-inferred");
        fs::write(root.join("real.rs"), "fn main() {}\n").unwrap();
        symlink(root.join("real.rs"), root.join("src/bin/worker.rs")).unwrap();
        let cargo: toml::Value = toml::from_str(
            r#"
            [package]
            name = "repo-worker"
            autobins = false

            [[bin]]
            name = "worker"
            "#,
        )
        .unwrap();

        let error = cargo_binary_names(&root, &cargo).unwrap_err();
        assert!(format!("{error:#}").contains("regular non-symlink file"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_bin_without_path_rejects_ambiguous_inferred_sources() {
        let root = temp_root("ambiguous-inferred");
        fs::write(root.join("src/bin/worker.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(root.join("src/bin/worker")).unwrap();
        fs::write(root.join("src/bin/worker/main.rs"), "fn main() {}\n").unwrap();
        let cargo: toml::Value = toml::from_str(
            r#"
            [package]
            name = "repo-worker"
            autobins = false

            [[bin]]
            name = "worker"
            "#,
        )
        .unwrap();

        let error = cargo_binary_names(&root, &cargo).unwrap_err();
        assert!(format!("{error:#}").contains("ambiguous inferred sources"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cargo_binary_names_reject_non_boolean_autobins() {
        let root = temp_root("malformed-autobins");
        let cargo: toml::Value = toml::from_str(
            r#"
            [package]
            name = "repo-worker"
            autobins = "false"
            "#,
        )
        .unwrap();

        let error = cargo_binary_names(&root, &cargo).unwrap_err().to_string();
        assert!(error.contains("autobins must be a boolean"));
        fs::remove_dir_all(root).unwrap();
    }
}
