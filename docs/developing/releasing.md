# Releasing telex

Maintainer runbook for cutting a public GitHub release. This documents the
*process*; the authoritative *mechanics* live in code and are linked below so this
page cannot silently drift from them:

- Release automation: [`.github/workflows/release.yml`](../../.github/workflows/release.yml)
- Installers: [`install.sh`](../../install.sh), [`install.ps1`](../../install.ps1)
- Machine-checked contract: [`tests/release_contract.rs`](../../tests/release_contract.rs)
  (runs in `cargo test`)

## How a release is produced

Pushing a `v*` tag triggers `release.yml`, which:

1. Verifies the tag matches the `Cargo.toml` version (fails fast on mismatch).
2. Builds the batteries-included binary (`--features entra`) for every target in
   the build matrix.
3. Packages each build as `telex-<tag>-<target>.{zip,tar.gz}` with a sibling
   `<asset>.sha256` checksum.
4. After **all** matrix legs succeed, a single `publish` job verifies that every
   archive has a paired `.sha256` sidecar (so the fail-closed in-binary
   `telex upgrade` never meets a sidecar-less release), then creates the GitHub
   release and uploads every asset at once.

The build matrix (the supported targets), the archive grammar, and the checksum
format are defined in `release.yml` and asserted by `tests/release_contract.rs` —
consult those rather than a copy here.

`workflow_dispatch` runs the same build matrix **without publishing** (the
`verify-version` and `publish` jobs require a `refs/tags/v*` **push**, so a manual
dispatch never publishes -- even if pointed at a tag ref). Asset names are derived
from a slash-sanitized ref, so a dispatch works against any pushed branch, including
`release/vX.Y.Z`. Use it to validate builds before tagging.

## Supported platforms

The release builds the targets the installers know how to fetch: x86_64 and
aarch64 Windows, x86_64 Linux, and x86_64 + aarch64 macOS. `tests/release_contract.rs`
enforces that every installer-requested target is built by the workflow.

**ARM Linux (`aarch64-unknown-linux-gnu`) is intentionally not shipped as a
prebuilt asset.** `install.sh` reports it as unsupported and directs users to
`cargo install --git https://github.com/lossyrob/telex --features entra`. If demand
warrants it, adding `aarch64-unknown-linux-gnu` (on an `ubuntu-24.04-arm` runner)
to the matrix and an install.sh case arm is a self-contained follow-up.

## Tag and version convention

- Tags are `vX.Y.Z`; the `Cargo.toml` `version` is `X.Y.Z` (no `v`). The workflow's
  `verify-version` job enforces the match.
- Telex is pre-1.0; use `0.MINOR.PATCH`.
- Several version strings must move together with `Cargo.toml`:
  - `Cargo.toml` `version` (and refresh `Cargo.lock`)
  - `.github/plugin/marketplace.json` (`metadata.version` and the plugin `version`)
  - `copilot/plugin/plugin.json` (`version`)
  - the `--plugin-version` example in `copilot/plugin/skills/telex/SKILL.md`

  (The plugin/binary compatibility check is version-matched, so drift here surfaces
  to users. A future improvement is to derive these from a single source.)

## Pre-cut checklist

Run through this before pushing a tag:

- [ ] `git switch main && git pull` — release from an up-to-date `main`.
- [ ] Bump `Cargo.toml` `version` to the release version; run a build so `Cargo.lock`
      updates; commit.
- [ ] Bump the plugin/marketplace version strings listed above to match.
- [ ] `cargo test --workspace` is green (includes `tests/release_contract.rs`).
- [ ] Trigger a `workflow_dispatch` run of **Release** and confirm all matrix legs
      build/package/checksum/upload artifacts — pay attention to the
      `aarch64-pc-windows-msvc` leg (`--features entra` on ARM Windows).
      `workflow_dispatch` runs against a **pushed ref**, so push the release commit
      to `main` (or a `release/vX.Y.Z` branch) first, then dispatch against it. A
      dispatch run exercises **build + package + checksum + upload only** — the
      `verify-version` and `publish` jobs are tag-gated and run only on the real
      `git push origin vX.Y.Z`, so nothing is published by a dispatch run.
- [ ] Prepare release notes (see below); skim `git log --oneline` for the range
      since the last tag.
- [ ] One-time: confirm **Settings > Code security > Private vulnerability
      reporting** is enabled on the repository (referenced by `SECURITY.md`).
- [ ] Review the **version axes** below and update
      `tests/fixtures/release/version-axes-previous.json` to record this release's values
      before starting the next one: set every entry under `axes` to the value this release
      ships, set `recorded_for_release` to this release's tag, and reset every entry under
      `expected_movement` to `"unchanged"` (an axis that has not moved *yet* in the next
      release is by definition unchanged). `tests/release_contract.rs` asserts the
      relationship the fixture declares, so performing this step keeps the suite green;
      the next change to an axis flips its `expected_movement` to `"changed"` (or
      `"introduced"` for a brand-new axis) in the same commit that bumps the constant.

## Version axes

Four independent version axes ship with the binary. They are *not* the crate version, and each moves
for its own reason. `tests/release_contract.rs` asserts each against a frozen previous-release
fixture (`tests/fixtures/release/version-axes-previous.json`), so an axis that is supposed to stay
put is checked against a **recorded** value rather than against a second copy of the current
constant — which would assert nothing.

The fixture carries an `expected_movement` map (`unchanged` / `changed` / `introduced`) alongside
the recorded values, and the test asserts that relationship rather than a hardcoded current value.
That is what makes the checklist step above executable: rolling the fixture forward and resetting
`expected_movement` leaves CI green, and the *next* commit that bumps an axis declares the movement
in the same change.

| Axis | Where | Bump when |
|---|---|---|
| `STATION_INTENT_SCHEMA_VERSION` | `src/station_intent.rs` | the on-disk station-intent manifest shape changes. Widen `_MIN_SUPPORTED` / `_MAX_SUPPORTED` deliberately: an intent outside the supported range is reported `incompatible` and never acted on. |
| `COPILOT_BRIDGE_PROTOCOL` | `src/commands/copilot.rs`, `copilot/bridge/probe-protocol.mjs` | the bridge wire contract changes (framing, verbs, request/response fields). A producer below `BRIDGE_PROBE_MIN_PROTOCOL` is classified *legacy*, not failed. |
| `PROTOCOL_MINOR` | `src/daemon_ipc.rs` | the daemon IPC surface gains a backward-compatible capability. Clients that require it gate on the minor (e.g. `RECONCILE_MIN_DAEMON_MINOR`). |
| `MIN_COMPATIBLE_PLUGIN_VERSION` | `src/commands/copilot.rs` | the **plugin** bootstrap contract (`copilot/plugin/hooks.json`, `copilot/plugin/plugin.json`) changes in a way an older plugin cannot satisfy. A bridge-extension-only change does **not** move this axis. |

Bumping any axis means updating the fixture in the *following* release, not the current one: the
fixture always records the previous release.

### Scheduled removals

- **`.bindings.json`** (`~/.copilot/telex-bridge/<session>.bindings.json`) is retained only as the
  extension teardown ref-count. Station intents (ADR 0050) are authoritative for recovery and for
  `telex copilot gc` keep decisions. Remove `.bindings.json` in the release **after** the one that
  ships station intents, once no supported binary still writes it.

## First release (v0.1.0)

1. Complete the pre-cut checklist. There is no previous tag, so the
   `workflow_dispatch` validation is the only pre-publish signal — do not skip it.
2. Tag and push:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. Watch the **Release** workflow. The `publish` job runs only after every build
   leg succeeds, so a single failed platform blocks the release rather than
   publishing a partial asset set.

### Release notes on the first release

`release.yml` uses `generate_release_notes: true`. On the **first** release there is
no previous tag, so GitHub generates notes from the entire commit history, which is
long and includes internal reconcile/merge commits. Before announcing:

- Edit the generated release on GitHub and replace the body with a concise,
  curated summary (highlights, install instructions, known limitations), **or**
- Draft the notes ahead of time and paste them in after the workflow creates the
  release.

Subsequent releases (with a previous tag) generate a bounded, useful changelog.

## Post-cut verification

Confirm a clean install from the published assets on each platform family:

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/lossyrob/telex/main/install.sh | sh
telex version --json    # current_tag should be the release you cut
```

```powershell
# Windows
irm https://raw.githubusercontent.com/lossyrob/telex/main/install.ps1 | iex
telex version --json
```

Verify a checksum was published alongside each archive (the installers verify it
automatically when present) and that `telex version --json` reports the expected
`version.install.current_tag`.

## Rollback / hotfix

If a published release is broken:

1. Delete the release and tag:

   ```sh
   gh release delete v0.1.0 --yes
   git push --delete origin v0.1.0
   ```

2. Fix the problem on `main`.
3. **Cut a new patch version — never re-use a tag.** Re-pushing an identical tag
   does not reliably re-trigger the workflow, and users who pinned `TELEX_VERSION`
   or hit cached `Latest` state can see stale assets. Bump to `v0.1.1` and cut it
   through the normal flow.

Users who already installed the bad release recover by re-running the install
script (which installs the new `Latest`) or `telex upgrade`.

## Install URL contract

The documented one-liners and both installers hard-code `REPO="lossyrob/telex"` and
fetch `install.{sh,ps1}` from `.../lossyrob/telex/main/...`. These URLs become an
external contract the moment a release exists. If the repository is ever renamed or
transferred, update **both** installers' `REPO` constant and every documented URL
(README, install guide, this runbook) in a single change, and post a redirect note,
because `raw.githubusercontent.com` redirects are unreliable.
