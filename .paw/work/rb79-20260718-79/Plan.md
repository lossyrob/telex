# Plan

## Approach Summary

Keep compatibility ownership in the binary while covering the stale-binary failure that occurs before Rust command dispatch. Add a build identifier to the binary's version and versioned-install metadata so same-semver builds can be distinguished, with backward-compatible reads for older manifests and binaries. Route only the Copilot `agentStop` drain hook through small platform launchers that invoke the binary-owned `copilot drain` command and translate an invocation failure into an actionable `agentStop` block decision; successful drains retain the binary's existing fail-open behavior.

## Work Items

- [x] Add a binary build identifier to `telex version`, versioned install manifests, and upgrade metadata while accepting older metadata that lacks it.
- [x] Add cross-platform drain hook launchers that honor `TELEX_COPILOT_DRAIN`, suppress successful adapter output, and surface missing/stale binary failures as actionable Copilot continuation prompts.
- [x] Update plugin contract tests and directly related compatibility documentation, then validate the targeted and repository test surfaces.

## Key Decisions

- The plugin does not define a compatibility matrix or duplicate binary workflow instructions; it only reports that its required binary command could not be invoked.
- Only command-surface incompatibility blocks `agentStop`. Once `copilot drain` starts, its existing bounded, fail-open runtime semantics remain unchanged.
- The failure prompt names the version/build diagnostic and versioned upgrade path, plus the existing drain off-switch as an explicit temporary escape hatch.
- Build metadata is additive and backward-compatible: new binaries can still inspect or install older binaries/manifests that do not report a build identifier.

## Open Questions

None.
