## Summary

If a sandbox create request is cancelled or times out on the client side while the node-side create is still in progress, the API reports the create as failed and never registers the sandbox. The node still holds the created instance. It is not in the running store, the team index, or the catalog.

An existing safety net reclaims it: `Store.Reconcile` → `killOrphanSandbox` kills instances present on the node but absent from the store, after the orphan grace period (`orphanGracePeriod = time.Minute`). So the window is bounded, but during that window an unregistered VM is running, and the create failure path itself performs no compensation.

## Root cause

The node-side create and the API-side registration are separate steps, and the failure path only cleans up after a *successful* placement.

In `packages/api/internal/orchestrator/create_instance.go`, `CreateSandbox` calls `placement.PlaceSandbox`. When the request context is cancelled, the placement loop classifies the failure as a timeout and returns before any `sandboxStore.Add`. The gRPC `Create` already sent to the node is not cancelled atomically with the API request, so the node completes the create and records the instance, while the API returns an error and never registers it:

```
ERROR Failed to create sandbox {"sandbox.id":"<id>", "node.id":"<node>", "error":"[Canceled] context canceled"}
WARN  error when creating instance "... failed to place sandbox: request timed out ..."
```

The cleanup that removes a node instance (`removeSandboxFromNode`) only runs on the `sandboxStore.Add` failure branch, i.e. after a successful placement; the placement-failure branch does not compensate. The instance is then only reclaimed by the periodic reconcile, one grace period later:

```
WARN Killing orphaned sandbox not found in store   (Store.Reconcile -> killOrphanSandbox)
```

## Reproduction

1. Start a create that takes longer than the client-side timeout (for example a cold/large template on a loaded host), with a short client timeout:

   ```sh
   curl -sS --max-time 1 -X POST "$API/sandboxes" \
     -H "X-API-Key: $KEY" -H 'Content-Type: application/json' \
     -d '{"templateID":"<template>","timeout":3600}'
   # curl: (28) Operation timed out
   ```

2. The API logs the cancellation for the corresponding sandbox id (`Failed to create sandbox ... "[Canceled] context canceled"`).

3. Observe the node holding the instance while it is absent from the running store and team index:

   ```
   node instance set:   [ ..., "<id>" ]     # present on the node
   redis running store: [ ... ]             # "<id>" absent
   team index SET:      [ ... ]             # "<id>" absent
   API GET /sandboxes/<id> -> 404
   ```

4. Roughly one orphan grace period later, the API logs `Killing orphaned sandbox not found in store` and the instance disappears from the node.

## Suggested fix

Compensate on the placement-failure branch instead of relying on the periodic reconcile:

- Record the intended `(sandboxID, executionID)` before dispatching the create, and on any failure after the node may have received the create — including cancellation/timeout — issue a best-effort `removeSandboxFromNode` for that exact id.
- Alternatively, make node-side create self-cleaning within a bounded window if the caller never confirms registration.

## Impact

For up to one orphan grace period, a VM and its network slot are running without any running-store record, any index entry, or any API visibility, and without the create failure path attempting compensation. It is reclaimed by `Store.Reconcile`, so it is a bounded leak rather than a permanent one.

Related: #2813 (orphaned Firecracker instances / leftover network rules), #3193 (node has no independent sandbox timeout enforcement), #1498 (failed startup leaves a sandbox running).