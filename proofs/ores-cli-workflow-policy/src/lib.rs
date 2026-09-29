//! Independent conformance oracle for the merged ORES workflow-policy audit.
//!
//! This is deliberately not a copy of `ores-cli` implementation code. It
//! exercises the externally intended admission semantics with a small,
//! dependency-free state machine so source and oracle cannot pass by sharing
//! the same parser implementation.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: &'static str,
    pub job: Option<String>,
}

#[derive(Debug, Default)]
struct JobState {
    name: String,
    has_runs_on: bool,
    has_steps: bool,
    has_job_uses: bool,
    has_timeout: bool,
}

pub fn audit_workflow(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    audit_permissions(text, &mut findings);
    audit_jobs(text, &mut findings);
    audit_uses(text, &mut findings);
    findings
}

fn audit_permissions(text: &str, findings: &mut Vec<Finding>) {
    let top_level = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with('#')
                && line.len() == trimmed.len()
                && trimmed.starts_with("permissions:")
        })
        .count();

    if top_level != 1 {
        findings.push(Finding {
            code: "workflow-permissions-top-level-invalid",
            job: None,
        });
    }
}

fn audit_jobs(text: &str, findings: &mut Vec<Finding>) {
    let mut in_jobs = false;
    let mut active = None::<JobState>;

    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len().saturating_sub(trimmed.len());

        if indent == 0 {
            if trimmed == "jobs:" {
                finish_job(active.take(), findings);
                in_jobs = true;
                continue;
            }
            if in_jobs {
                finish_job(active.take(), findings);
                in_jobs = false;
            }
            continue;
        }
        if !in_jobs {
            continue;
        }

        if indent == 2 && is_job_key(trimmed) {
            finish_job(active.take(), findings);
            active = Some(JobState {
                name: trimmed.trim_end_matches(':').to_owned(),
                ..JobState::default()
            });
            continue;
        }

        let Some(job) = active.as_mut() else {
            continue;
        };
        if indent != 4 {
            continue;
        }

        if trimmed.starts_with("runs-on:") {
            job.has_runs_on = true;
        } else if trimmed == "steps:" {
            job.has_steps = true;
        } else if trimmed.starts_with("uses:") {
            job.has_job_uses = true;
        } else if trimmed.starts_with("timeout-minutes:") {
            job.has_timeout = true;
        }
    }

    finish_job(active, findings);
}

fn finish_job(job: Option<JobState>, findings: &mut Vec<Finding>) {
    let Some(job) = job else {
        return;
    };

    if job.has_job_uses {
        if job.has_timeout {
            findings.push(Finding {
                code: "workflow-reusable-job-timeout-invalid",
                job: Some(job.name),
            });
        }
        return;
    }

    if (job.has_runs_on || job.has_steps) && !job.has_timeout {
        findings.push(Finding {
            code: "workflow-job-timeout-missing",
            job: Some(job.name),
        });
    }
}

fn audit_uses(text: &str, findings: &mut Vec<Finding>) {
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        let candidate = trimmed.strip_prefix('-').map_or(trimmed, str::trim_start);
        let Some(value) = candidate.strip_prefix("uses:") else {
            continue;
        };
        let reference = value
            .trim()
            .split_once(" #")
            .map_or(value.trim(), |(head, _)| head.trim());

        if !immutable_uses(reference) {
            findings.push(Finding {
                code: "workflow-action-ref-mutable",
                job: None,
            });
        }
    }
}

fn immutable_uses(reference: &str) -> bool {
    if reference.starts_with("./") {
        return !reference.contains('@');
    }
    if let Some((name, digest)) = reference
        .strip_prefix("docker://")
        .and_then(|value| value.rsplit_once("@sha256:"))
    {
        return !name.is_empty() && is_lower_hex(digest, 64);
    }

    let Some((source, revision)) = reference.rsplit_once('@') else {
        return false;
    };
    source.split('/').count() >= 2
        && !source.chars().any(char::is_whitespace)
        && (is_lower_hex(revision, 40) || is_lower_hex(revision, 64))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_job_key(trimmed: &str) -> bool {
    let Some(name) = trimmed.strip_suffix(':') else {
        return false;
    };
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::audit_workflow;

    const CHECKOUT: &str = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1";

    #[test]
    fn hardened_executable_job_passes() {
        let workflow = format!(
            "name: CI\npermissions:\n  contents: read\njobs:\n  test:\n    runs-on: ubuntu-24.04\n    timeout-minutes: 10\n    steps:\n      - uses: {CHECKOUT}\n"
        );
        assert!(audit_workflow(&workflow).is_empty());
    }

    #[test]
    fn executable_job_requires_timeout() {
        let workflow = "name: CI\npermissions: read-all\njobs:\n  test:\n    runs-on: ubuntu-24.04\n    steps: []\n";
        let findings = audit_workflow(workflow);
        assert!(findings.iter().any(|finding| {
            finding.code == "workflow-job-timeout-missing"
                && finding.job.as_deref() == Some("test")
        }));
    }

    #[test]
    fn reusable_workflow_call_is_timeout_exempt() {
        let workflow = "name: Reuse\npermissions: read-all\njobs:\n  policy:\n    uses: owner/repo/.github/workflows/policy.yml@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
        assert!(audit_workflow(workflow).is_empty());
    }

    #[test]
    fn reusable_workflow_call_rejects_timeout_key() {
        let workflow = "name: Reuse\npermissions: read-all\njobs:\n  policy:\n    uses: owner/repo/.github/workflows/policy.yml@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n    timeout-minutes: 5\n";
        let findings = audit_workflow(workflow);
        assert!(findings.iter().any(|finding| {
            finding.code == "workflow-reusable-job-timeout-invalid"
                && finding.job.as_deref() == Some("policy")
        }));
    }

    #[test]
    fn step_level_uses_does_not_turn_job_into_reusable_call() {
        let workflow = format!(
            "name: CI\npermissions: read-all\njobs:\n  test:\n    runs-on: ubuntu-24.04\n    timeout-minutes: 10\n    steps:\n      - uses: {CHECKOUT}\n"
        );
        assert!(audit_workflow(&workflow).is_empty());
    }

    #[test]
    fn mutable_action_tag_is_rejected() {
        let workflow = "name: CI\npermissions: read-all\njobs:\n  test:\n    runs-on: ubuntu-24.04\n    timeout-minutes: 10\n    steps:\n      - uses: actions/checkout@v4\n";
        let findings = audit_workflow(workflow);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "workflow-action-ref-mutable")
        );
    }

    #[test]
    fn missing_top_level_permissions_is_rejected() {
        let workflow = "name: CI\njobs:\n  test:\n    runs-on: ubuntu-24.04\n    timeout-minutes: 10\n    steps: []\n";
        let findings = audit_workflow(workflow);
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "workflow-permissions-top-level-invalid")
        );
    }

    #[test]
    fn local_and_digest_actions_are_accepted() {
        let workflow = "name: CI\npermissions: read-all\njobs:\n  test:\n    runs-on: ubuntu-24.04\n    timeout-minutes: 10\n    steps:\n      - uses: ./actions/local\n      - uses: docker://ghcr.io/example/tool@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n";
        assert!(audit_workflow(workflow).is_empty());
    }
}
