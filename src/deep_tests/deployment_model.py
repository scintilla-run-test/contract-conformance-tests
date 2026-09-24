from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass


class DeploymentConflict(ValueError):
    pass


class StaleRuntimeEpoch(ValueError):
    pass


@dataclass(frozen=True)
class DeploymentCommand:
    kind: str
    deployment_id: str | None
    runtime_epoch: int
    idempotency_key: str
    archive_sha256: str | None = None

    def fingerprint(self) -> str:
        body = json.dumps(
            {
                "archive_sha256": self.archive_sha256,
                "deployment_id": self.deployment_id,
                "kind": self.kind,
                "runtime_epoch": self.runtime_epoch,
            },
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        return hashlib.sha256(body).hexdigest()


@dataclass(frozen=True)
class DeploymentReceipt:
    revision: int
    runtime_epoch: int
    active_deployment_id: str | None
    observed: bool
    operation: str


class DeploymentReferenceModel:
    """Small oracle for immutable deployment-generation lifecycle semantics.

    The model deliberately separates a runtime epoch (placement/process lifetime)
    from immutable deployment identities. A code-only generation change activates
    another known artifact inside the same epoch. Placement failover advances the
    epoch and clears the active generation until an exact activation is observed.
    """

    def __init__(self, runtime_epoch: int = 1) -> None:
        if runtime_epoch < 1:
            raise ValueError("runtime_epoch must be positive")
        self._runtime_epoch = runtime_epoch
        self._revision = 0
        self._artifacts: dict[str, str] = {}
        self._active_deployment_id: str | None = None
        self._dedupe: dict[str, tuple[str, DeploymentReceipt]] = {}
        self._history: list[DeploymentReceipt] = []

    @property
    def runtime_epoch(self) -> int:
        return self._runtime_epoch

    @property
    def active_deployment_id(self) -> str | None:
        return self._active_deployment_id

    @property
    def history(self) -> tuple[DeploymentReceipt, ...]:
        return tuple(self._history)

    def apply(self, command: DeploymentCommand) -> DeploymentReceipt:
        fingerprint = command.fingerprint()
        prior = self._dedupe.get(command.idempotency_key)
        if prior is not None:
            prior_fingerprint, prior_receipt = prior
            if prior_fingerprint != fingerprint:
                raise DeploymentConflict(
                    "idempotency key was reused for a different deployment intent"
                )
            return prior_receipt

        if command.kind == "register":
            receipt = self._register(command)
        elif command.kind in {"activate", "rollback"}:
            receipt = self._activate(command)
        elif command.kind == "advance_epoch":
            receipt = self._advance_epoch(command)
        else:
            raise ValueError(f"unknown deployment command kind: {command.kind}")

        self._dedupe[command.idempotency_key] = (fingerprint, receipt)
        self._history.append(receipt)
        return receipt

    def snapshot(self) -> str:
        return json.dumps(
            {
                "active_deployment_id": self._active_deployment_id,
                "artifacts": dict(sorted(self._artifacts.items())),
                "revision": self._revision,
                "runtime_epoch": self._runtime_epoch,
            },
            sort_keys=True,
            separators=(",", ":"),
        )

    def _register(self, command: DeploymentCommand) -> DeploymentReceipt:
        self._require_current_epoch(command.runtime_epoch)
        deployment_id = _require_digest(command.deployment_id, "deployment_id")
        archive_sha256 = _require_digest(command.archive_sha256, "archive_sha256")
        prior_archive = self._artifacts.get(deployment_id)
        if prior_archive is not None and prior_archive != archive_sha256:
            raise DeploymentConflict(
                "immutable deployment_id was rebound to different artifact bytes"
            )
        self._artifacts[deployment_id] = archive_sha256
        return self._receipt("register", observed=True)

    def _activate(self, command: DeploymentCommand) -> DeploymentReceipt:
        self._require_current_epoch(command.runtime_epoch)
        deployment_id = _require_digest(command.deployment_id, "deployment_id")
        if deployment_id not in self._artifacts:
            raise DeploymentConflict("deployment generation is not registered")
        self._active_deployment_id = deployment_id
        return self._receipt(command.kind, observed=True)

    def _advance_epoch(self, command: DeploymentCommand) -> DeploymentReceipt:
        if command.deployment_id is not None or command.archive_sha256 is not None:
            raise ValueError("advance_epoch does not accept deployment bytes")
        if command.runtime_epoch != self._runtime_epoch + 1:
            raise StaleRuntimeEpoch(
                "runtime epoch must advance monotonically by exactly one"
            )
        self._runtime_epoch = command.runtime_epoch
        self._active_deployment_id = None
        return self._receipt("advance_epoch", observed=True)

    def _require_current_epoch(self, runtime_epoch: int) -> None:
        if runtime_epoch != self._runtime_epoch:
            raise StaleRuntimeEpoch(
                f"expected runtime epoch {self._runtime_epoch}, got {runtime_epoch}"
            )

    def _receipt(self, operation: str, observed: bool) -> DeploymentReceipt:
        self._revision += 1
        return DeploymentReceipt(
            revision=self._revision,
            runtime_epoch=self._runtime_epoch,
            active_deployment_id=self._active_deployment_id,
            observed=observed,
            operation=operation,
        )


def _require_digest(value: str | None, field: str) -> str:
    if value is None:
        raise ValueError(f"{field} is required")
    normalized = value.removeprefix("sha256:")
    if len(normalized) != 64 or any(ch not in "0123456789abcdef" for ch in normalized):
        raise ValueError(f"{field} must be a lowercase SHA-256 digest")
    return f"sha256:{normalized}"
