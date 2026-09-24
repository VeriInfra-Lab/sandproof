## Summary

If a sandbox is paused and a `DELETE` arrives while a `resume` for the same sandbox is already in flight, the API returns:

- `DELETE /sandboxes/{id}` → **204** (success, and the paused snapshot representation is soft-deleted), and
- `POST /sandboxes/{id}/resume` → **201**,

after which `GET /sandboxes/{id}` reports **`running`**. The client is told the sandbox was deleted, but the sandbox is alive.

## Root cause

A paused sandbox has no running-store record: its only representation is a snapshot row. That produces two independent problems:

1. **Kill has nowhere to record its intent.** `DELETE` calls `RemoveSandbox` → `sandboxStore.StartRemoving`, which reads the running store and returns `ErrNotFound` because there is no record. No tombstone or removal marker is written. The handler then proceeds to `deleteSnapshot` (`packages/api/internal/handlers/sandbox_kill.go`), which soft-deletes the snapshot env (`UPDATE envs SET deleted_at = NOW()`) and returns 204.

2. **Resume publishes unconditionally.** A `resume` that already passed its snapshot fetch spends seconds in node restore. When it completes, `CreateSandbox` calls `sandboxStore.Add`, which is a lockless, unconditional `SET`+`SADD` (`packages/api/internal/sandbox/storage/redis/operations.go`, source comment: *"Add is lockless"*). It does not check for a delete intent — none was recorded — so the sandbox is published as running.

This is the same class of stale-decision race that was fixed elsewhere in the storage layer: writes there are atomic (Lua scripts, per-sandbox locks, `ExpectExecutionID`), and the catalog uses server-side compare-and-delete. `Add` is the one publication point that is neither locked nor guarded.

## Reproduction

The race is in the API control plane and does not depend on a particular backend. Trigger it by deleting while a resume is still restoring:

1. Create a sandbox and pause it:
   ```
   POST /sandboxes            -> 201
   POST /sandboxes/{id}/pause -> 204   (snapshot written)
   ```
2. Start a resume and, while it is still in flight (the restore takes seconds for a cold/large snapshot), issue the delete:
   ```
   POST /sandboxes/{id}/resume   (async)
      DELETE /sandboxes/{id}     -> 204   (snapshot soft-deleted, running record absent)
      resume completes           -> 201
   GET /sandboxes/{id}           -> 200 state=running
   ```
3. The client received `204` for the delete, yet the sandbox is running again.

Deterministic across repeated runs. Control: with no resume in flight, the same `DELETE` returns 204 and `GET` returns 404 — so the race between delete and the in-flight resume publication is the cause, not paused-delete itself.

## Suggested fix

Record a removal tombstone even when there is no running record, and make publication check it atomically:

1. On `DELETE` of a paused sandbox, write a per-sandbox removal marker atomically (a Redis key, or a `deleted_at`/tombstone on the snapshot row) together with deleting the snapshot.
2. Replace the lockless `Add` `SET`+`SADD` with a server-side script that rejects the write when the sandbox has a tombstone (or when it is not in an acceptable state), mirroring `startTransitionScript` and the catalog compare-and-delete.
3. Alternatively, make `resume`'s final publication take the same per-sandbox lock as `StartRemoving` and re-check the current state before publishing.

An accepted kill must be irreversible: a concurrent resume must not be able to publish afterward.

## Impact

A user-facing `DELETE` reports success while the sandbox continues to run — and its snapshot has been deleted — breaking kill semantics and leaving the sandbox running until its timeout, with the client believing it is gone.