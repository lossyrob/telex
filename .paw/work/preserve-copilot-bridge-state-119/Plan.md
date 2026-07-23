# Plan: Preserve Copilot Bridge State

## Approach

Align the filesystem lifecycle with the existing non-destructive daemon lifecycle. A successful `telex copilot session-end` will continue to mark the current store's membership idle and clear transient turn-guard state, but it will no longer delete the session-scoped bridge extension, bridge registry, or address bindings. Existing explicit cleanup paths will remain unchanged.

## Key Decisions

- Treat ordinary session end as resumable, not as implicit detach.
- Preserve all durable bridge artifacts on successful session end; `copilot resume` remains responsible for re-arming daemon push registration and rescanning unacknowledged backlog.
- Keep final-binding detach, push-to-pull fallback transition, provisioning rollback, and GC destructive.
- Add process-level coverage using an isolated Copilot home so the test proves both artifact retention after session end and artifact removal after explicit detach.
- Update the version-matched Copilot skill and operator/design documentation so startup discovery is the normal resume path and `extensions_reload` is limited to first-time or recovery provisioning into an already-running session.

## Work Items

1. **Implement and document the resumable bridge lifecycle**
   - Remove session-end bridge teardown while retaining transient turn-guard cleanup.
   - Add process-level regression coverage that provisions a bridge, records a registry fixture, runs session end, verifies the member is idle and all bridge artifacts remain, resumes the member, and verifies explicit final-binding detach removes the artifacts.
   - Update bridge code comments, the version-matched Copilot skill, the user guide, and the bridge design notes to distinguish startup discovery from live-session reload recovery.
   - Run targeted Rust tests and documentation-sensitive tests, then run repository formatting/checks appropriate to the changed surfaces.

## Success Criteria

- Successful `copilot session-end` preserves `extension.mjs`, `busy-state.mjs`, the binding file, and registry entry.
- Session-end still marks only the selected store's member idle and clears turn-guard state.
- `copilot resume` re-arms the preserved member.
- Explicit final-binding detach still removes the extension directory, binding file, and registry entry.
- Fallback, rollback, and GC cleanup behavior remains covered and unchanged.
- Copilot skill and push-delivery documentation accurately describe normal startup discovery and limited `extensions_reload` usage.
