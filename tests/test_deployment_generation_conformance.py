import unittest

from deep_tests.deployment_model import (
    DeploymentCommand,
    DeploymentConflict,
    DeploymentReferenceModel,
    StaleRuntimeEpoch,
)


D1 = "sha256:" + "1" * 64
D2 = "sha256:" + "2" * 64
A1 = "sha256:" + "a" * 64
A2 = "sha256:" + "b" * 64


def command(
    kind: str,
    key: str,
    deployment_id: str | None = None,
    runtime_epoch: int = 1,
    archive_sha256: str | None = None,
) -> DeploymentCommand:
    return DeploymentCommand(
        kind=kind,
        deployment_id=deployment_id,
        runtime_epoch=runtime_epoch,
        idempotency_key=key,
        archive_sha256=archive_sha256,
    )


class DeploymentGenerationConformanceTests(unittest.TestCase):
    def test_code_only_generation_change_preserves_runtime_epoch(self) -> None:
        model = DeploymentReferenceModel(runtime_epoch=7)
        model.apply(command("register", "r1", D1, 7, A1))
        first = model.apply(command("activate", "a1", D1, 7))
        model.apply(command("register", "r2", D2, 7, A2))
        second = model.apply(command("activate", "a2", D2, 7))

        self.assertEqual(first.runtime_epoch, 7)
        self.assertEqual(second.runtime_epoch, 7)
        self.assertEqual(second.active_deployment_id, D2)
        self.assertTrue(second.observed)

    def test_rollback_reactivates_known_immutable_generation(self) -> None:
        model = DeploymentReferenceModel()
        model.apply(command("register", "r1", D1, 1, A1))
        model.apply(command("register", "r2", D2, 1, A2))
        model.apply(command("activate", "a1", D1))
        model.apply(command("activate", "a2", D2))
        rolled_back = model.apply(command("rollback", "rb1", D1))

        self.assertEqual(rolled_back.active_deployment_id, D1)
        self.assertEqual(rolled_back.runtime_epoch, 1)
        self.assertEqual(rolled_back.operation, "rollback")
        self.assertTrue(rolled_back.observed)

    def test_exact_idempotent_replay_returns_same_receipt(self) -> None:
        model = DeploymentReferenceModel()
        register = command("register", "same", D1, 1, A1)
        first = model.apply(register)
        second = model.apply(register)

        self.assertEqual(first, second)
        self.assertEqual(len(model.history), 1)

    def test_idempotency_key_cannot_be_reused_for_other_generation(self) -> None:
        model = DeploymentReferenceModel()
        model.apply(command("register", "r1", D1, 1, A1))
        model.apply(command("register", "r2", D2, 1, A2))
        model.apply(command("activate", "activation", D1))

        with self.assertRaises(DeploymentConflict):
            model.apply(command("activate", "activation", D2))

    def test_same_deployment_identity_cannot_be_rebound_to_other_bytes(self) -> None:
        model = DeploymentReferenceModel()
        model.apply(command("register", "r1", D1, 1, A1))

        with self.assertRaises(DeploymentConflict):
            model.apply(command("register", "r2", D1, 1, A2))

    def test_unknown_generation_cannot_be_activated(self) -> None:
        model = DeploymentReferenceModel()
        with self.assertRaises(DeploymentConflict):
            model.apply(command("activate", "a1", D1))

    def test_stale_epoch_cannot_activate_or_register(self) -> None:
        model = DeploymentReferenceModel(runtime_epoch=3)
        for kind in ("register", "activate"):
            with self.subTest(kind=kind), self.assertRaises(StaleRuntimeEpoch):
                model.apply(
                    command(
                        kind,
                        f"{kind}-stale",
                        D1,
                        runtime_epoch=2,
                        archive_sha256=A1 if kind == "register" else None,
                    )
                )

    def test_failover_advances_epoch_and_requires_fresh_activation_ack(self) -> None:
        model = DeploymentReferenceModel(runtime_epoch=4)
        model.apply(command("register", "r1", D1, 4, A1))
        model.apply(command("activate", "a1", D1, 4))
        advanced = model.apply(command("advance_epoch", "epoch-5", runtime_epoch=5))

        self.assertEqual(advanced.runtime_epoch, 5)
        self.assertIsNone(advanced.active_deployment_id)
        activated = model.apply(command("activate", "a2", D1, 5))
        self.assertEqual(activated.active_deployment_id, D1)
        self.assertTrue(activated.observed)

    def test_epoch_may_not_skip_or_move_backwards(self) -> None:
        model = DeploymentReferenceModel(runtime_epoch=9)
        for epoch in (8, 9, 11):
            with self.subTest(epoch=epoch), self.assertRaises(StaleRuntimeEpoch):
                model.apply(command("advance_epoch", f"epoch-{epoch}", runtime_epoch=epoch))

    def test_snapshot_is_deterministic_and_binds_active_generation(self) -> None:
        model = DeploymentReferenceModel()
        model.apply(command("register", "r2", D2, 1, A2))
        model.apply(command("register", "r1", D1, 1, A1))
        model.apply(command("activate", "a1", D1))

        expected = (
            '{"active_deployment_id":"%s","artifacts":{"%s":"%s","%s":"%s"},'
            '"revision":3,"runtime_epoch":1}'
            % (D1, D1, A1, D2, A2)
        )
        self.assertEqual(model.snapshot(), expected)


if __name__ == "__main__":
    unittest.main()
