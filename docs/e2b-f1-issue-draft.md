> Submitted: https://github.com/e2b-dev/runtime/issues/3636 (`e2b-dev/infra` renamed to `e2b-dev/runtime`).
>
> Tested revision: `82f6529660ea98087d7a479247c789905b0a3867`; the involved code paths are unchanged on `main` (`767ceb4b2ec0e598f512767c8da9e5e6618da368`, only unrelated `errors.AsType` refactors). Not verified against the hosted service.

## Summary

If a sandbox is paused and a `DELETE` arrives while a `resume` for the same sandbox is already in flight, the API returns:

- `DELETE /sandboxes/{id}` → **204** (success, and the paused snapshot representation is soft-deleted), and
- `POST /sandboxes/{id}/resume` → **201**,

after which `GET /sandboxes/{id}` reports **`running`**. The client is told the sandbox was deleted, but the sandbox is alive.

## Root cause

A paused sandbox has no running-store record: its only representation is a snapshot row in Postgres. That produces two independent problems:

1. **Kill has nowhere to record its intent.** `DELETE` calls `RemoveSandbox` → `sandboxStore.StartRemoving`, which reads the running store and returns `ErrNotFound` because there is no record. No tombstone or removal marker is written. The handler then proceeds to `deleteSnapshot` (`packages/api/internal/handlers/sandbox_kill.go`), which soft-deletes the snapshot env (`UPDATE envs SET deleted_at = NOW()`) and returns 204.

2. **Resume publishes unconditionally.** A `resume` that already passed its snapshot fetch spends seconds in node restore. When it completes, `CreateSandbox` calls `sandboxStore.Add`, which is a lockless, unconditional `SET`+`SADD` (`packages/api/internal/sandbox/storage/redis/operations.go`, source comment: *"Add is lockless"*). It does not check for a delete intent — none was recorded — so the sandbox is published as running.

This is the same class of stale-decision race that was previously fixed elsewhere in the storage layer: writes there are atomic (Lua scripts, per-sandbox locks, `ExpectExecutionID`), and the catalog uses server-side compare-and-delete. `Add` is the one publication point that is neither locked nor guarded.

## Reproduction (no Firecracker required)

The race is entirely in the API control plane, so it reproduces on macOS with the official `dummy-orchestrator`:

1. Run Postgres + Redis, migrate and seed the DB (as in `tests/integration`).
2. Run the api (`ENVIRONMENT=local`, local orchestrator discovery) and the dummy orchestrator on `:5008`.
3. Hold the dummy's `Create` RPC for a few seconds (`time.Sleep` on an env-configured duration) so the node-restore window is wide enough to interleave.

Then:

```
POST /sandboxes                      -> 201 (running)
POST /sandboxes/{id}/pause           -> 204 (paused; snapshot row written)
POST /sandboxes/{id}/resume   (async)
   2s later: DELETE /sandboxes/{id}  -> 204   (deletes the snapshot env)
   resume completes                  -> 201
GET /sandboxes/{id}                  -> 200 state=running
```

Observed deterministically across 3/3 runs. Control: with no resume in flight, the same `DELETE` returns 204 and `GET` returns 404 — so the race, not paused-kill itself, is the cause.

Direct DB observation in a single run (snapshot env identified from the `snapshots` row):

```
before  envs(<snapshot env>).deleted_at = NULL   active_envs = 1
DELETE  204
during  envs(<snapshot env>).deleted_at = <now>  active_envs = 0
resume  201
final   GET state=running   redis running_key EXISTS = 1
```

I can provide the reproducer script, the dummy-orchestrator failpoint patch, and raw logs.

## Reproduction harness details

Everything runs locally on macOS, no Firecracker/KVM:

1. `postgres:18` (db `mydatabase`) and `redis:8` via Docker; run `make -C packages/db migrate`; seed with `tests/integration/seed.go` (synthetic team/API key/template/build).
2. Build and run `packages/api` with `ENVIRONMENT=local` (local orchestrator discovery → `127.0.0.1:5008`).
3. Build and run the official `packages/orchestrator/cmd/dummy-orchestrator`, with two small local-only edits:
   - `dummyserver.SandboxServer.Create`: `time.Sleep` for `DUMMY_CREATE_DELAY_MS` (widens the node-restore window);
   - `dummyserver.InfoServer.ServiceInfo`: report non-zero `MetricCpuCount`/memory (otherwise the dummy scores as unplaceable).

Two caveats for honesty: the node is the official dummy (the race is in the API control plane, so this does not affect it), and the delay is a local failpoint that widens the window from milliseconds to seconds.

## Suggested fix

Record a removal tombstone even when there is no running record, and make publication check it atomically.

1. On `DELETE` of a paused sandbox, write a per-sandbox removal marker atomically (e.g. a Redis key, or a `deleted_at`/tombstone on the snapshot row) before/with deleting the snapshot.
2. Replace the lockless `Add` `SET`+`SADD` with a server-side script that rejects the write when the sandbox has a tombstone (or when it is not in `paused`/absent), mirroring `startTransitionScript` and the catalog compare-and-delete.
3. Alternatively/additionally, make `resume`'s final publication take the same per-sandbox lock as `StartRemoving` and re-check the current state before publishing.

The second option's contract should mirror the model used elsewhere: an accepted kill is irreversible, so a concurrent resume must not be able to publish afterward.

## Impact

A user-facing `DELETE` reporting success while the sandbox continues to run — and its snapshot has been deleted — breaks the expected kill semantics and can leave the sandbox running until its timeout, with the client believing it is gone.
