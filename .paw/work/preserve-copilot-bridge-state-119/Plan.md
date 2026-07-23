# Plan: Preserve Copilot Bridge State

## Approach

Align the filesystem lifecycle with the existing non-destructive daemon lifecycle. A successful `telex copilot session-end` will continue to mark the current store's membership idle and clear transient turn-guard state, but it will no longer delete the session-scoped bridge extension, bridge registry, or address bindings. Existing explicit cleanup paths will remain unchanged.

## Key Decisions

- Treat ordinary session end as resumable, not as implicit detach.
- Preserve all durable bridge artifacts on successful session end; `copilot resume` remains responsible for re-arming daemon push registration and rescanning unacknowledged backlog.
- Keep final-binding detach, push-to-pull fallback transition, provisioning rollback, and GC destructive.
- Add process-level coverage using an isolated Copilot home so the test proves the retain/remove split, resume re-registration, backlog rescan, and explicit cleanup behavior.
- Keep the daemon's existing definite-end semantics in scope as an observed constraint: resume may perform a full re-registration and emit the existing session-id-reuse diagnostic, but must still succeed.
- Update the version-matched Copilot skill and operator/design documentation so startup discovery is the normal resume path. Limit `extensions_reload` guidance to first-time provisioning or recovery provisioning into an already-running session, including the case where a retained extension is not live after startup.
- Document that an ended session intentionally retains resumable bridge files until explicit detach or GC reclaims a session that will not resume.

## Work Items

1. **Implement and document the resumable bridge lifecycle**
   - Remove session-end bridge teardown while retaining transient turn-guard cleanup.
   - Add process-level regression coverage that provisions a bridge, records a registry fixture and transient turn-guard state, runs session end, verifies the member is idle, verifies turn-guard state is cleared, and verifies all durable bridge artifacts remain.
   - Resume the same session and assert the member is active with push registration restored. Arrange unacknowledged backlog or push-attempt state and prove resume schedules a fresh push attempt/backlog sweep. Allow the daemon's existing definite-end/session-id-reuse diagnostic while requiring successful re-registration.
   - Verify explicit final-binding detach removes the extension directory, binding file, and registry. Extend targeted fallback, failed-provisioning rollback, and GC coverage where needed so each destructive exception proves complete bridge-artifact cleanup rather than only daemon state or one file.
   - Update stale lifecycle comments in `src/commands/copilot.rs` and `copilot/bridge/extension.mjs`, the version-matched Copilot skill, the user guide, and the bridge design notes. Distinguish startup discovery from first-time/live-session recovery reloads, and explain detach/forced-GC reclamation for sessions that will not resume.
   - Run targeted Rust tests and documentation-sensitive tests, then run repository formatting/checks appropriate to the changed surfaces.

## Success Criteria

- Successful `copilot session-end` preserves `extension.mjs`, `busy-state.mjs`, the binding file, and registry entry.
- Session-end still marks only the selected store's member idle and clears turn-guard state.
- `copilot resume` succeeds after definite session end, restores active push registration, and triggers a fresh scan/attempt for queued unacknowledged backlog.
- Explicit final-binding detach still removes the extension directory, binding file, and registry entry.
- Fallback transition, failed provisioning rollback, and GC each retain complete destructive cleanup coverage.
- Copilot skill and push-delivery documentation accurately describe normal startup discovery, restrict `extensions_reload` to first-time or already-running-session recovery, and explain explicit reclamation of retained state.
