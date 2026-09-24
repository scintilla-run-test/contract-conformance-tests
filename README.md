# scintilla-run-test/contract-conformance-tests

Deterministic state-model, idempotency, serialization, and protocol contract conformance tests.

This repository is the `contract` deep-test suite for `scintilla-run`. It is intentionally dependency-light and deterministic so failures can be reproduced locally without production credentials or customer data.

The suite also contains an implementation-neutral deployment-generation oracle. It proves that immutable artifact identities cannot be rebound to different bytes, code-only activation preserves the current runtime epoch, stale epochs are rejected, idempotency keys cannot be reused for different intent, failover advances the epoch before reactivation, and rollback reactivates a previously admitted immutable generation rather than mutating it.

## Run

```bash
PYTHONPATH=src python -m unittest discover -s tests -v
python scripts/verify_repository.py
```

The initial models are executable rather than placeholders. Product adapters should be added through focused pull requests while preserving the reference-model tests as an oracle.

Tracking: https://github.com/ORESoftware/ai-agent-coordinator.rs/issues/139
