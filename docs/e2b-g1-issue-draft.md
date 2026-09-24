## Summary

If a sandbox create request is cancelled or times out on the client side while the node-side create is still in progress, the API reports the create as failed, but the node keeps the created instance. Because the sandbox was never registered in the running store, it never appears in the team index or the expiration index, so no cleanup path (`ExpiredItems`, `Remove`) ever sees it. The instance survives as an orphan until the host is restarted.

## Root cause

The node-side create and the API-side registration are separate steps, and the failure path only cleans up after a *successful* placement.

In `packages/api/internal/orchestrator/create_instance.go`, `CreateSandbox` calls `placement.PlaceSandbox`. When the request context is cancelled, the placement loop classifies the failure as a timeout and returns before any `sandboxStore.Add`:

```
WARN error when creating instance ... "failed to place sandbox: request timed out after 1 placement attempt(s)"
```

The gRPC `Create` already sent to the node is not cancelled atomically with the API request: the node completes the create and records the instance, while the API returns an error and never registers it. The API even logs the sandbox id for the cancelled attempt together with the cancellation:

```
ERROR Failed to create sandbox {"sandbox.id":"<id>", "node.id":"<node>", "error":"[Canceled] context canceled"}
```

The cleanup that removes a node instance (`removeSandboxFromNode`) only runs on the `sandboxStore.Add` failure branch, i.e. after a successful placement. There is no equivalent cleanup on the placement-failure branch, because at that point the API has not constructed the `sandbox.Sandbox` value and does not track the node instance that may have been created.

Two further facts make the orphan permanent:

1. The evictor works from the running store / expiration ZSET. A sandbox that was never added has no entry, so `ExpiredItems()` cannot find or reclaim it.
2. The sandbox is absent from the team index and the sandbox catalog, so listing and routing do not surface it either.

## Reproduction

1. Start a create that takes longer than the client-side timeout — for example a cold/large template on a loaded host, or any host where the node create takes several seconds. Issue it with a short client timeout:

   ```sh
   curl -sS --max-time 1 -X POST "$API/sandboxes" \
     -H "X-API-Key: $KEY" -H 'Content-Type: application/json' \
     -d '{"templateID":"<template>","timeout":600}'
   # -> curl: (28) Operation timed out
   ```

2. The API logs the cancellation for the corresponding sandbox id:

   ```
   ERROR Failed to create sandbox {"sandbox.id":"<id>", "node.id":"<node>", "error":"[Canceled] context canceled"}
   WARN  error when creating instance "... failed to place sandbox: request timed out ..."
   ```

3. Observe the node still holds the instance that the API reported as failed. The instance is present on the node, but absent from the Redis running store and the team index:

   ```
   node sandboxes:        [ ..., "<id>" ]         # instance exists on the node
   redis running store:   [ ... ]                 # "<id>" is not present
   team index SET:        [ ... ]                 # "<id>" is not present
   ```

   The instance remains after the create has long returned, with no running-store record and no expiration-index entry.

## Suggested fix

Reclaim the node instance on the placement-failure branch, not just on the `Add`-failure branch:

- Record the intended `(sandboxID, executionID)` before dispatching the create, and on any failure after the node may have received the create — including context cancellation/timeout — issue a best-effort `removeSandboxFromNode` for that exact id (the delete is already keyed by sandbox id and validated against the node's lifecycle id, so it is safe to attempt).
- Alternatively, make node-side create idempotent and self-cleaning: if the caller never confirms registration within a bounded window, the node reclaims the instance. This also matches the existing request for node-independent cleanup (`#3193`) and reduces the orphan classes reported in `#2813`.

## Impact

Each cancelled/timed-out create can leave one VM, its rootfs/snapshot resources, a network slot and iptables rules on the host with no owner. They are invisible to the evictor and to listing, consume host capacity, and are only reclaimed by a host restart.

Related: #2813 (orphaned Firecracker instances / leftover network rules), #3193 (node has no independent sandbox timeout enforcement), #1498 (failed startup leaves a sandbox running).

---

Submitted: https://github.com/e2b-dev/runtime/issues/3637
