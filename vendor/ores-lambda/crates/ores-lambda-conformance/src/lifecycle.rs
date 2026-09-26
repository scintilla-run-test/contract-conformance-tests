use anyhow::{bail, Context};
use std::{collections::BTreeSet, fs, path::Path};
use toml::{Table, Value};

const SOURCE_LIFECYCLE_FILES: &[&str] = &[
    ".zpkg.toml",
    "zed-env.toml",
    ".zed/pre-install",
    ".zed/post-install",
    ".githooks/pre-commit",
    ".githooks/pre-push",
    "conformance/check.sh",
    "scripts/install-git-hooks.sh",
    "scripts/check-bootstrap.sh",
    "scripts/check-contracts.sh",
    "scripts/check-conformance.sh",
    "scripts/check-fast.sh",
    "scripts/check-post-install.sh",
    "scripts/check.sh",
    "scripts/check-package.sh",
    "generated/README.md",
];

const PACKAGE_LIFECYCLE_FILES: &[&str] = &[
    ".zpkg.toml",
    "conformance/check.sh",
    "scripts/check-package.sh",
];

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let (contract_count, conformance_file_count) = check_authorities(root)?;
    for path in SOURCE_LIFECYCLE_FILES {
        require_regular_file(root, path)?;
    }

    let manifest = load_toml(&root.join(".zpkg.toml"))?;
    check_package_manifest(&manifest)?;
    let environment = load_toml(&root.join("zed-env.toml"))?;
    check_zed_environment(&manifest, &environment)?;
    check_wrapper_wiring(root)?;
    check_gitignore(root)?;

    println!(
        "ores-lambda source lifecycle admission passed: package=oresoftware/ores-lambda contracts={contract_count} peers={} conformance_files={conformance_file_count} checker=conformance/check.sh",
        contract_count * 2,
    );
    Ok(())
}

pub(crate) fn check_package(root: &Path) -> anyhow::Result<()> {
    let (contract_count, conformance_file_count) = check_authorities(root)?;
    for path in PACKAGE_LIFECYCLE_FILES {
        require_regular_file(root, path)?;
    }
    let manifest = load_toml(&root.join(".zpkg.toml"))?;
    check_package_manifest(&manifest)?;
    println!(
        "ores-lambda packaged lifecycle admission passed: contracts={contract_count} peers={} conformance_files={conformance_file_count}",
        contract_count * 2,
    );
    Ok(())
}

fn check_authorities(root: &Path) -> anyhow::Result<(usize, usize)> {
    let contract_count = check_contract_inventory(root)?;
    let conformance_file_count = check_conformance_inventory(root)?;
    check_canonical_zed_checker(root)?;
    Ok((contract_count, conformance_file_count))
}

fn check_contract_inventory(root: &Path) -> anyhow::Result<usize> {
    let directory = root.join("contracts");
    let metadata = fs::symlink_metadata(&directory).context("contracts/ is missing")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("contracts/ must be a real directory, not a symlink")
    }

    let mut typespec = BTreeSet::new();
    let mut schema = BTreeSet::new();
    for entry in fs::read_dir(&directory).context("read contracts/")? {
        let entry = entry.context("read contracts/ entry")?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name
            .to_str()
            .context("contracts/ filenames must be UTF-8")?;
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect contracts/{file_name}"))?;
        if metadata.file_type().is_symlink() {
            bail!("contract authority must not be a symlink: contracts/{file_name}")
        }
        if metadata.is_dir() {
            bail!("nested contract directories are not admitted: contracts/{file_name}")
        }
        if !metadata.is_file() {
            bail!("contract authority must be a regular file: contracts/{file_name}")
        }

        if file_name == "README.md" {
            continue;
        }
        if let Some(stem) = file_name.strip_suffix(".schema.json") {
            if stem.is_empty() || !schema.insert(stem.to_owned()) {
                bail!("duplicate or empty JSON Schema authority stem: {file_name}")
            }
        } else if let Some(stem) = file_name.strip_suffix(".tsp") {
            if stem.is_empty() || !typespec.insert(stem.to_owned()) {
                bail!("duplicate or empty TypeSpec authority stem: {file_name}")
            }
        } else {
            bail!("unexpected file in contracts/: {file_name}")
        }
    }

    if typespec.is_empty() {
        bail!("contracts/ must contain at least one TypeSpec/JSON Schema peer pair")
    }
    if typespec != schema {
        let missing_schema = typespec.difference(&schema).cloned().collect::<Vec<_>>();
        let missing_typespec = schema.difference(&typespec).cloned().collect::<Vec<_>>();
        bail!(
            "contract peer inventory mismatch: missing_schema={missing_schema:?} missing_typespec={missing_typespec:?}"
        )
    }
    Ok(typespec.len())
}

fn check_conformance_inventory(root: &Path) -> anyhow::Result<usize> {
    let directory = root.join("conformance");
    let metadata = fs::symlink_metadata(&directory).context("conformance/ is missing")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("conformance/ must be a real directory, not a symlink")
    }

    let mut count = 0usize;
    visit_conformance_directory(&directory, &directory, &mut count)?;
    if count < 2 {
        bail!("conformance/ must contain a canonical checker plus executable fixtures/vectors")
    }
    Ok(count)
}

fn visit_conformance_directory(
    base: &Path,
    directory: &Path,
    count: &mut usize,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("read conformance directory {}", directory.display()))?
    {
        let entry = entry.context("read conformance entry")?;
        let path = entry.path();
        let relative = path.strip_prefix(base).unwrap_or(&path);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect conformance/{}", relative.display()))?;
        if metadata.file_type().is_symlink() {
            bail!(
                "conformance authority must not be a symlink: conformance/{}",
                relative.display()
            )
        }
        if metadata.is_dir() {
            visit_conformance_directory(base, &path, count)?;
        } else if metadata.is_file() {
            *count += 1;
        } else {
            bail!(
                "conformance authority must be a regular file: conformance/{}",
                relative.display()
            )
        }
    }
    Ok(())
}

fn check_canonical_zed_checker(root: &Path) -> anyhow::Result<()> {
    // Current Zed checker priority is check.mjs, check.js, check.sh, .... This
    // repository intentionally owns check.sh. A new higher-priority checker must
    // therefore fail review instead of silently replacing the audited authority.
    for higher_priority in ["conformance/check.mjs", "conformance/check.js"] {
        match fs::symlink_metadata(root.join(higher_priority)) {
            Ok(_) => bail!(
                "{higher_priority} would shadow canonical conformance/check.sh under Zed checker priority"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspect {higher_priority}"));
            }
        }
    }
    require_regular_file(root, "conformance/check.sh")
}

fn require_regular_file(root: &Path, relative: &str) -> anyhow::Result<()> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("required lifecycle file missing: {relative}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("required lifecycle authority must be a regular non-symlink file: {relative}");
    }
    Ok(())
}

fn load_toml(path: &Path) -> anyhow::Result<Value> {
    let input = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&input).with_context(|| format!("parse {}", path.display()))
}

fn table<'a>(value: &'a Value, key: &str, owner: &str) -> anyhow::Result<&'a Table> {
    value
        .get(key)
        .and_then(Value::as_table)
        .with_context(|| format!("{owner} is missing [{key}]"))
}

fn string_field<'a>(table: &'a Table, key: &str, owner: &str) -> anyhow::Result<&'a str> {
    table
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("{owner}.{key} must be a string"))
}

fn check_package_manifest(manifest: &Value) -> anyhow::Result<()> {
    let package = table(manifest, "package", ".zpkg.toml")?;
    for (field, expected) in [
        ("org", "oresoftware"),
        ("name", "ores-lambda"),
        ("language", "rust"),
    ] {
        let actual = string_field(package, field, ".zpkg.toml [package]")?;
        if actual != expected {
            bail!(".zpkg.toml package.{field} must be {expected:?}, got {actual:?}");
        }
    }

    let dependencies = table(manifest, "dependencies", ".zpkg.toml")?;
    let tjsv = dependencies
        .get("oresoftware/typespec-json-schema-validator")
        .and_then(Value::as_str)
        .context(
            ".zpkg.toml must resolve oresoftware/typespec-json-schema-validator through zed-pkg",
        )?;
    if tjsv.is_empty() || tjsv == "*" {
        bail!("zed TJSV dependency must use a non-empty reviewed version constraint")
    }

    let install = table(manifest, "install", ".zpkg.toml")?;
    if string_field(install, "dir", ".zpkg.toml [install]")? != ".vendor/.zed" {
        bail!(".zpkg.toml install.dir must be .vendor/.zed")
    }

    let targets = table(manifest, "targets", ".zpkg.toml")?;
    for (target, expected_dir, expected_adapter) in [
        ("rust", ".", "rust"),
        ("contracts", "contracts", "none"),
        ("conformance", "conformance", "none"),
    ] {
        let target_table = targets
            .get(target)
            .and_then(Value::as_table)
            .with_context(|| format!(".zpkg.toml is missing [targets.{target}]"))?;
        let actual_dir = string_field(target_table, "dir", &format!("[targets.{target}]"))?;
        let actual_adapter = string_field(target_table, "adapter", &format!("[targets.{target}]"))?;
        if actual_dir != expected_dir || actual_adapter != expected_adapter {
            bail!(
                "[targets.{target}] must use dir={expected_dir:?} adapter={expected_adapter:?}; got dir={actual_dir:?} adapter={actual_adapter:?}"
            )
        }
    }

    let scripts = table(manifest, "scripts", ".zpkg.toml")?;
    if scripts.len() != 1 || !scripts.contains_key("test") {
        bail!(".zpkg.toml [scripts] must expose only the package-level test hook");
    }
    if string_field(scripts, "test", ".zpkg.toml [scripts]")? != "sh scripts/check.sh" {
        bail!(".zpkg.toml scripts.test must delegate to scripts/check.sh");
    }

    let publish = table(manifest, "publish", ".zpkg.toml")?;
    let smoke = string_field(publish, "smoke_test", ".zpkg.toml [publish]")?;
    if !smoke.contains("scripts/check-package.sh") {
        bail!(".zpkg.toml publish.smoke_test must execute scripts/check-package.sh");
    }
    if let Some(excludes) = publish.get("exclude").and_then(Value::as_array) {
        for exclude in excludes.iter().filter_map(Value::as_str) {
            let normalized = exclude.trim_start_matches("./");
            if normalized == "contracts"
                || normalized.starts_with("contracts/")
                || normalized == "conformance"
                || normalized.starts_with("conformance/")
            {
                bail!("publish.exclude must not remove contract/conformance authority: {exclude}")
            }
        }
    }
    Ok(())
}

fn check_zed_environment(manifest: &Value, environment: &Value) -> anyhow::Result<()> {
    if environment.get("schema").and_then(Value::as_integer) != Some(2) {
        bail!("zed-env.toml must use schema = 2");
    }
    let tasks = table(environment, "tasks", "zed-env.toml")?;
    for task in [
        "bootstrap",
        "contracts",
        "conformance",
        "precommit",
        "prepush",
        "package",
        "test",
    ] {
        if !tasks.get(task).is_some_and(Value::is_table) {
            bail!("zed-env.toml is missing [tasks.{task}]");
        }
    }

    require_task_command(tasks, "bootstrap", "sh scripts/check-bootstrap.sh")?;
    require_task_command(tasks, "contracts", "sh scripts/check-contracts.sh")?;
    require_task_command(tasks, "conformance", "sh scripts/check-conformance.sh")?;
    require_task_command(tasks, "precommit", "sh .githooks/pre-commit")?;
    require_task_command(tasks, "prepush", "sh .githooks/pre-push")?;
    require_task_command(tasks, "package", "sh scripts/check-package.sh")?;

    let package_test = table(manifest, "scripts", ".zpkg.toml")?
        .get("test")
        .and_then(Value::as_str)
        .context(".zpkg.toml scripts.test must be a string")?;
    let task_test = task_commands(tasks, "test")?.join(" && ");
    if package_test != task_test {
        bail!("Zed package test and zed-env tasks.test drifted: {package_test:?} != {task_test:?}");
    }
    Ok(())
}

fn require_task_command(tasks: &Table, task: &str, expected: &str) -> anyhow::Result<()> {
    let commands = task_commands(tasks, task)?;
    if !commands.iter().any(|command| command == expected) {
        bail!("zed-env.toml tasks.{task} must include {expected:?}");
    }
    Ok(())
}

fn task_commands(tasks: &Table, task: &str) -> anyhow::Result<Vec<String>> {
    let task = tasks
        .get(task)
        .and_then(Value::as_table)
        .with_context(|| format!("zed-env.toml is missing task {task}"))?;
    let run = task
        .get("run")
        .context("zed-env.toml task is missing run")?;
    match run {
        Value::String(command) if !command.is_empty() => Ok(vec![command.clone()]),
        Value::Array(commands) if !commands.is_empty() => commands
            .iter()
            .map(|command| {
                command
                    .as_str()
                    .filter(|command| !command.is_empty())
                    .map(str::to_owned)
                    .context("zed-env.toml task run arrays must contain non-empty strings")
            })
            .collect(),
        _ => bail!("zed-env.toml task run must be a non-empty string or string array"),
    }
}

fn check_wrapper_wiring(root: &Path) -> anyhow::Result<()> {
    for (path, needle) in [
        (".zed/pre-install", "sh scripts/check-bootstrap.sh"),
        (".zed/post-install", "sh scripts/check-post-install.sh"),
        (
            "scripts/check-post-install.sh",
            "zed validate --require-lock",
        ),
        (".githooks/pre-commit", "sh scripts/check-fast.sh"),
        (".githooks/pre-push", "zed install"),
        (".githooks/pre-push", "zed install --frozen"),
        (".githooks/pre-push", "zed build"),
        (".githooks/pre-push", "zed run test"),
        (".githooks/pre-push", "zed pack --out"),
        ("conformance/check.sh", "sh scripts/check-contracts.sh"),
        (
            "conformance/check.sh",
            "ores-lambda-conformance -- self-test",
        ),
        (
            "conformance/check.sh",
            "ores-lambda-ingress -- --check-config",
        ),
        ("scripts/check-contracts.sh", "zed run tjsv"),
        ("scripts/check-conformance.sh", "sh conformance/check.sh"),
        ("scripts/check-fast.sh", "sh conformance/check.sh"),
        ("scripts/check.sh", "sh scripts/check-fast.sh"),
        (
            "scripts/check-package.sh",
            "ores-lambda-conformance -- package",
        ),
        (
            "scripts/check-package.sh",
            "ores-lambda-conformance -- self-test",
        ),
    ] {
        let content =
            fs::read_to_string(root.join(path)).with_context(|| format!("read {path}"))?;
        if !content.contains(needle) {
            bail!("{path} must delegate to {needle}");
        }
    }
    Ok(())
}

fn check_gitignore(root: &Path) -> anyhow::Result<()> {
    let content = fs::read_to_string(root.join(".gitignore")).context("read .gitignore")?;
    for required in ["/.conformance/", "/.vendor/", "/.zed-pack/"] {
        if !content.lines().any(|line| line.trim() == required) {
            bail!(".gitignore must contain {required}");
        }
    }
    Ok(())
}
