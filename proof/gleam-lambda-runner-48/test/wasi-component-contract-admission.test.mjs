import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const admissionPath = new URL(
  '../contract-admission/scintilla-wasi-component.v1.json',
  import.meta.url,
);

async function readAdmission() {
  return JSON.parse(await readFile(admissionPath, 'utf8'));
}

test('WASI runtime consumes the canonical TJSV-admitted Scintilla contract', async () => {
  const admission = await readAdmission();

  assert.equal(admission.schema, 'scintilla.external-contract-admission/v1');
  assert.equal(
    admission.consumer_repository,
    'scintilla-run/gleam-lambda-runner',
  );
  assert.equal(admission.contract.id, 'scintilla.wasi-component/v1');
  assert.equal(
    admission.contract.authority_repository,
    'scintilla-run/scintilla-interfaces',
  );
  assert.match(admission.contract.authority_commit, /^[0-9a-f]{40}$/u);
  assert.match(admission.contract.typespec_blob, /^[0-9a-f]{40}$/u);
  assert.match(admission.contract.json_schema_blob, /^[0-9a-f]{40}$/u);
  assert.equal(admission.contract.authority_precedence, 'none');

  assert.equal(
    admission.validator.repository,
    'ORESoftware/typespec-json-schema-validator',
  );
  assert.equal(
    admission.validator.commit,
    '813d5f021e02574f529a9553b670719f8f10d02c',
  );
  assert.equal(
    admission.validator.contract_ir_schema,
    'ores.typespec-json-schema-validator.contract-ir/v1',
  );

  assert.equal(admission.admitted_evidence.status, 'passed');
  assert.equal(admission.admitted_evidence.zero_unexplained_findings, true);
  assert.match(admission.admitted_evidence.contract_ir_id, /^[0-9a-f]{64}$/u);
  assert.match(
    admission.admitted_evidence.parity_receipt_digest,
    /^[0-9a-f]{64}$/u,
  );
  assert.match(
    admission.admitted_evidence.consumer_verification_id,
    /^[0-9a-f]{64}$/u,
  );
  assert.equal(
    admission.admitted_evidence.proof_repository,
    'scintilla-run-test/contract-conformance-tests',
  );
  assert.equal(admission.admitted_evidence.proof_pr, 14);
  assert.match(
    admission.admitted_evidence.artifact_digest,
    /^sha256:[0-9a-f]{64}$/u,
  );
});

test('WASI admission fails closed instead of defining a second local authority', async () => {
  const admission = await readAdmission();

  assert.equal(admission.consumer_policy.allow_local_reauthoring, false);
  assert.equal(
    admission.consumer_policy.allow_generated_projection_as_authority,
    false,
  );
  assert.equal(admission.consumer_policy.require_exact_authority_commit, true);
  assert.equal(admission.consumer_policy.require_admissible_contract_ir, true);
  assert.equal(admission.consumer_policy.fail_closed_on_evidence_mismatch, true);
});
