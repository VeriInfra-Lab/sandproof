# Sandproof

**English** · [简体中文](README.zh-CN.md)

Sandproof is a stateful sandbox system implemented in Rust, with Lean proofs for selected control logic. Its current goal is reliable suspension, unloading, restoration on the same host, and resource reclamation despite management-service crashes, request retries, and concurrent operations.

## Current goals

The first stage focuses on three connected problems:

- **Preserve the latest state.** Capture full memory, VM/device state, and the matching private disk during the same freeze interval before unloading the old VM. Restoration must continue the original session's state.
- **Maintain exclusive execution authority.** Restore into a new execution incarnation. Old executors, duplicate requests, and late acknowledgments must not permit duplicate execution. Clients use operation keys to retrieve the result of the original request.
- **Reclaim recovery materials safely.** Release the execution slot after unloading while continuing to account for the session and its recovery materials. Materials still used by a restore operation or a running VM must not be deleted prematurely.

The system retains a real center-to-host network path: one active center writer with authoritative Redis, and one fixed Linux/KVM host with SQLite, supporting multiple users and sessions. The center owns global authorization, bindings, reservations, and intent; the host owns local control, persistence, physical execution, and precise cleanup. Redis, SQLite, files, and the VMM do not share an atomic transaction.

The [project plan (#1)](https://github.com/VeriInfra-Lab/sandproof/issues/1) and [stage1 blueprint](docs/project-blueprint.md) define the current scope. The new V0–V9 work is tracked in #632–#641, linked from the project plan. Delivery requires a working system, conditional proofs for the designated Rust implementation, and reproducible experiments covering state continuity, failures, and resources.

## Goals and implementation status

Existing center/host components, cold boot and cleanup, core E2B compatibility, the template pipeline, gateways, and shared proofs provide the foundation. **The new stateful lifecycle is being implemented and verified incrementally. The complete create → unload → management-process restart → resume → destroy path cannot yet be described as accepted or proved.** Pause/Resume types in a pure core, isolated component tests, and historical proofs do not establish acceptance of the actual product path.

This branch includes the current Stage1 design and committed implementation candidate. The lifecycle is incomplete and the full acceptance gates have not passed. This is a team development baseline, not a deployment release; exact proof applicability is recorded in [proof coverage](docs/proof-coverage.md).

The first stage requires full snapshots on the same host and loading a restored VM in a paused state. Suspension and restoration must not reset the session's total lifetime. New lifecycle operations require an operation key known to the client before its first send. Existing SDK create calls without a key do not automatically acquire safe retry semantics, and existing TCP/vsock connections are not guaranteed to survive restoration.

Cross-host migration, fork, incremental snapshots, object storage, center high availability, proofs of the complete guest protocol, and RAM recovery after arbitrary VMM death are outside this stage. Template builds and image conversion continue to rely on engineering tests. **Runtime snapshot management, restoration authority, and cleanup decisions are project-owned logic that must be verified in this stage.**

## Development entry points

Detailed documentation is currently primarily in Chinese. This English README is an entry point; it does not imply that all documents have English translations.

- [Project plan and tasks](https://github.com/VeriInfra-Lab/sandproof/issues/1), [milestones](https://github.com/VeriInfra-Lab/sandproof/milestones)
- [Contributing guide](CONTRIBUTING.md): branches, commits, PRs, testing, and documentation; task branch → integration → test → main
- [Documentation index for this branch](docs/README.md): integrated baseline documentation and tooling
- [Current stage blueprint](docs/project-blueprint.md), [architecture](docs/ARCHITECTURE.md), and [module contracts](docs/module-contracts.md): current goals, responsibilities, and failure semantics
- [Current verification design](docs/formal-verification-design.md), [proof coverage](docs/proof-coverage.md), and [acceptance plan](docs/testing-plan.md): target properties, applicability, and real-machine acceptance criteria
- [CI runs](https://github.com/VeriInfra-Lab/sandproof/actions)

## Code and proofs

| Directory | Purpose |
|---|---|
| `sandbox-center-core/`, `sandbox-center-store/` | Pure center decisions and persistent coordination through Redis |
| `controller-core/`, `controller-service/` | Local host control, SQLite, and recovery orchestration |
| `sandbox-node/`, `sandbox-runtime/` | Node protocol integration, physical resource execution, and precise cleanup |
| `sandbox-center/`, `sandbox-server/` | Assembly entry points for the center and protected host services |
| `SandboxModel/` | Lean models, properties, and correspondence proofs for designated Rust code |
| `verification/` | Pinned Charon/Aeneas extraction, toolchain documentation, and proof dependency audits |
| `scripts/`, `tools/` | Functional checks, proof acceptance, deployment, and development tools |
| `docs/`, `benchmarks/` | Integrated design and coverage records, plus pinned compatibility and benchmark entry points |

## Local checks

Follow the [contributing guide](CONTRIBUTING.md) and [CI environment instructions](docs/ci.md) to prepare the pinned toolchains, Python, and a verified Redis binary. Set `SANDPROOF_REDIS_SERVER` explicitly.

```sh
bash scripts/check-function.sh             # Complete Linux baseline checks
bash scripts/check-function.sh --portable  # macOS development checks; not Linux acceptance
```

Before proof development, read the entire [proof workflow](docs/proof-workflow.md) and use its designated service and pinned toolchain. The complete proof acceptance entry point is:

```sh
scripts/bootstrap-aeneas.sh  # Prepare the pinned toolchain on first use
scripts/verify-controller.sh
```

Lean is the source of the protocol specification. Charon/Aeneas extracts designated real Rust into Lean functions, whose correspondence with the specification is then proved. Extraction is limited by the pinned toolchain's supported subset and external contracts. Model theorems, implementation correspondence, native tests, and Linux/KVM experiments are reported separately. Existing results apply only to their recorded code, inputs, failure models, and assumptions; they do not establish complete verification of the system, virtualization isolation, or the underlying OS.
