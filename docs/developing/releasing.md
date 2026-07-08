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
4. After **all** matrix legs succeed, a single `publish` job creates the GitHub
   release and uploads every asset at once.

The build matrix (the supported targets), the archive grammar, and the checksum
format are defined in `release.yml` and asserted by `tests/release_contract.rs` —
consult those rather than a copy here.

`workflow_dispatch` runs the same build matrix **without publishing** (the
`verify-version` and `publish` jobs are gated to `refs/tags/v*`). Use it to
validate builds on a branch before tagging.

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
