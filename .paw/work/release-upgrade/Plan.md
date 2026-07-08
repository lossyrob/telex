# Plan — Release-based telex upgrade (release-upgrade, #60)

## Node Outcome Anchor

With a public release available, `telex upgrade` **with no `--from`** discovers a suitable
public GitHub release, selects the current-platform asset, downloads the archive + checksum,
verifies the checksum, then installs and switches through the existing versioned layout
(#6 / PR #56). `telex upgrade --version <tag>` installs an explicit public release.
Bad checksum, missing asset, unsupported platform, incompatible version, and already-current
cases fail closed or report clearly. Existing local `upgrade --from`, `rollback`, and `gc`
semantics keep working.

The plan must end with a **live, hermetic proof** that the release-fetch path downloads,
verifies, extracts, installs, and switches — not stop at prerequisite refactors.

## Approach Summary

Add an in-binary release-fetch path to `telex upgrade`. When `--from` is omitted, telex:

1. **Discovers** a release via the GitHub REST API — `/releases/latest` by default (which
   excludes drafts/prereleases), or `/releases/tags/<tag>` for explicit `--version`.
2. **Selects** the current platform asset by the release contract grammar
   `telex-<tag>-<target>.{zip,tar.gz}` plus its `<asset>.sha256` sidecar (from
   `.github/workflows/release.yml`, asserted by `tests/release_contract.rs`).
3. **Downloads** the archive and sidecar (reqwest), honoring an optional `GITHUB_TOKEN`
   for rate limits (matching `install.sh`/`install.ps1`).
4. **Verifies** the SHA-256 checksum before touching the install layout — fail closed on
   missing/mismatched checksum.
5. **Extracts** `telex(.exe)` from the archive (zip on Windows; tar.gz on Unix).
6. **Installs** through the existing `install::install_binary` + `switch_to` + drain flow,
   reusing manifests and compatibility checks unchanged. Source metadata is captured by
   running the extracted binary's `telex --json version` (existing `source_metadata()`).

The network path is compiled behind a **default-on `self-update` feature** so lean builds
that omit it get a clear "compiled without release upgrade; use `--from`" error rather than
a broken command. `upgrade --from <binary>` remains the manual/local path in all builds.

Discovery/download base URLs and repo are injectable (env overrides + `--repo`) so the
end-to-end proof runs against a **local fixture HTTP server** with zero external network.

## Key Decisions

- **KD1 — Feature gating (`self-update`, default-on).** New network deps live behind a
  `self-update` feature in `default` and implied wherever the shipped binary is built
  (release builds `--features entra`; `self-update` is independent and default-on, so the
  shipped binary always has it). Lean `--no-default-features` builds without it get an
  actionable error on `telex upgrade` (no `--from`). Rationale: keep `telex upgrade`'s
  release path available in every shipped binary without forcing the full network+archive
  stack into intentionally-lean builds.

- **KD2 — HTTP client + TLS stack (council-decided, HIGH confidence).** Use `reqwest` with
  `default-features = false` and explicit `native-tls` plus only the required transfer
  features. This reuses the openssl/schannel/Security.framework stack `postgres`/`native-tls`
  already pulls into default builds — avoids adding a *second* active TLS stack (rustls) to
  default non-entra builds; the `entra` build already carries both stacks so it is free there.
  *Alternative (minority, deferred):* `rustls` for musl/static portability; not adopted because
  the only beneficiaries are lean `sqlite`+`self-update`+musl *source* builds, which can compile
  the updater out. See Reopen Conditions. The updater path's TLS backend should be pinned/checked
  so it cannot silently drift to rustls.

- **KD3 — Fail closed on a missing checksum (council-decided) + machine-enforce the premise.**
  The in-binary upgrade **requires** a verified checksum by default; a missing sidecar is an
  error. **No escape hatch initially** (prefer a *loud env-var* hatch over a CLI flag if ever
  needed). *Planning-review correction (premortem MF1):* `fail_on_unmatched_files: true` only
  asserts the publish glob matched ≥1 file — it does **not** guarantee every archive has a paired
  `.sha256`. So the fail-closed-no-hatch stance is hardened by **adding a `release.yml` step
  (before publish) that fails if the sidecar count ≠ archive count** (each archive has a sibling
  `.sha256`), coupled in `tests/release_contract.rs`. This makes the "always publishes sidecars"
  premise actually machine-enforced rather than assumed. (In-scope per the streamliner guidance:
  a workflow adjustment directly required to make the fail-closed in-binary upgrade safe.)

- **KD7 — Security hardening (council + planning-review).** (a) HTTPS-only, including on
  redirects — reject a downgrade to plain HTTP for the real GitHub hosts (the injectable test
  base may be plain HTTP against loopback only). *(premortem SF-e)* Do **not** forward the
  `Authorization`/`GITHUB_TOKEN` header across a cross-host redirect — send the token only to
  the configured API host. (b) **Reject archive path traversal** (zip-slip / tar `..` / absolute
  / symlink-escape) via a **single `safe_extract(kind, archive, expected_name, out_dir)` helper**
  shared by both formats *(retrospective C4)* — only the expected `telex(.exe)` entry is written
  to a controlled temp dir. (c) **Stage before promote** — download → verify checksum → extract
  to temp, and only then hand a verified binary to `install::install_binary`. On **Unix set mode
  `0o755`** on the staged binary before it is executed *(premortem MF2)*. (d) **Strict sidecar
  parse** — field 1, lowercase hex of the exact SHA-256 length, exact compare. (e) Pin the reqwest
  TLS backend (`native-tls`, no rustls) so it cannot silently drift; a CI feature-combo build
  guards it. (f) The checksum is verified **before** the downloaded binary is ever executed
  (`source_metadata` runs only on a checksum-verified, staged binary); the residual trust root is
  the *repository* the asset came from — see KD6/`--repo` hardening. Cryptographic release signing
  (authenticity beyond integrity) is out of scope and noted as deferred.

- **KD4 — Platform target is compile-time (`cfg`), coupled to the release matrix.** The
  current target triple + archive kind are derived from `cfg(target_os/target_arch)`.
  Unsupported platforms fail closed with the same "use cargo install" guidance as
  `install.sh`. A contract test asserts the module's target set is a subset of
  `release.yml`'s build matrix (mirrors the existing installer-subset test) so a matrix
  change breaks a repo test, not a user's upgrade.

- **KD5 — already-current / incompatible reuse existing semantics + tag normalization.**
  Already-current: resolved tag == `version_info().install.current_tag` and no `--force` → clear
  no-op, exit 0. Incompatible: the existing `validate_manifest_for_current` (schema window +
  protocol major) runs before switch and produces the incompatible-version error. `--force`
  re-installs the same tag. *(retrospective SF-j)* Introduce `normalize_tag(&str) -> Result<String>`
  that enforces a `v` prefix + semver shape, applied at the CLI boundary **and** the already-current
  compare, so `--version 0.1.0` and `--version v0.1.0` behave identically and the compare against a
  stored `current_tag` cannot silently mismatch on the `v` prefix.

- **KD6 — Testability + `--repo` as a hidden/operator knob.** Discovery/download bases are
  injectable via env overrides `TELEX_UPGRADE_API_BASE` / `TELEX_UPGRADE_DOWNLOAD_BASE` and a repo
  override. *(premortem SF-c / retrospective `--repo`)* The repo override is **hidden**
  (`#[arg(hide = true)]`) and/or `TELEX_UPGRADE_REPO`, documented as test/enterprise-only — not a
  promoted user-facing flag — to avoid immortalizing `lossyrob/telex` into a phishing template and
  widening the trust surface (a `--repo attacker/telex` can serve a self-consistent poisoned asset;
  the checksum check does not defend a different repo). The end-to-end integration test packages the
  test's own built `telex` (`CARGO_BIN_EXE_telex`) into a real archive, serves a **byte-for-byte
  captured** `/releases/latest` + asset-list JSON (recorded from the real `v0.1.0` release) plus the
  archive + sidecar from a local `TcpListener`, and drives a full download→verify→extract→install→
  switch against a temp root. Recording real JSON *(retrospective SF-i)* catches parse-side GitHub
  drift on re-record. Pure helpers (asset selection, sidecar parse, checksum verify, target mapping,
  tag normalization) are unit-tested with no network.

## Work Items

- [x] **WI1 — Dependencies & `self-update` feature.** Add `self-update` to `default`; declare
  `reqwest` (`default-features = false`, `native-tls`, only required transfer features — **no
  rustls**), `sha2`, `zip`, `tar`, `flate2` under the feature; refresh `Cargo.lock`. Verify
  `cargo build` (default) and `cargo build --features entra` both succeed. Extend the CI
  `feature-check` job with `--no-default-features --features sqlite` (updater compiled out) and
  `--no-default-features --features "sqlite,self-update"` (musl reopen combo) so the compiled-out
  fallback and the lean self-update combo both stay green *(retrospective C3)*.
  `lite-task:release-upgrade:deps-feature`

- [x] **WI2 — `src/release.rs` release-fetch module** (behind `self-update`): target/archive
  detection; `normalize_tag` (`v`-prefix + semver) *(SF-j)*; serde types for GitHub release/asset
  JSON; pure helpers (`select_asset`, `parse_sha256_sidecar` [strict lowercase-hex, field 1],
  `verify_checksum`, `current_target`); async `discover_release` (latest / by-tag) and `download`
  (reqwest native-tls, `GITHUB_TOKEN`, User-Agent, HTTPS-only incl. redirects, **no auth header on
  cross-host redirect**, base-URL/repo injectable); a single **`safe_extract(kind, archive,
  expected_name, out_dir)`** helper for both formats that writes only `telex(.exe)`, **rejects path
  traversal**, and on Unix sets mode `0o755` on the staged binary *(MF2)*. Fail-closed error
  taxonomy (network, 403 rate-limit/auth, checksum, missing asset/sidecar, unsupported platform).
  `lite-task:release-upgrade:release-module`

- [x] **WI3 — CLI wiring (`src/cli.rs`).** `UpgradeArgs.from` → `Option<PathBuf>`; add a **hidden**
  repo override (`#[arg(hide = true)]`, default `lossyrob/telex`, also honoring `TELEX_UPGRADE_REPO`)
  and `--force`; keep `--version/--root/--no-switch/--skip-drain/--drain-timeout-ms`. Update the
  dispatch and the existing CLI parse tests. `lite-task:release-upgrade:cli`

- [x] **WI4 — Upgrade command branch (`src/commands/upgrade.rs`).** Branch on `--from`:
  present → existing local path; absent → release path (discover→download→verify→extract→
  reuse `install_binary`/`switch_to`/drain). Normalize the resolved tag *(SF-j)*; already-current
  short-circuit; incompatible-version and all fail-closed diagnostics surfaced as **actionable**
  errors (network, auth/rate-limit with a `GITHUB_TOKEN` hint, checksum, missing asset/sidecar,
  unsupported platform, already-current). JSON output includes resolved tag / asset / verified /
  source. Clear compiled-out error when `self-update` is absent. `lite-task:release-upgrade:command`

- [x] **WI5 — Tests.** Unit tests for pure helpers incl. `normalize_tag` and traversal rejection
  (no network). Integration test `tests/release_upgrade.rs`: local fixture server end-to-end happy
  path (**the live proof**) driven from a byte-for-byte captured real `/releases/latest` payload,
  selecting the per-platform archive kind so **both** Windows (zip) and Linux (tar.gz) extraction +
  Unix exec-bit are exercised by the existing ubuntu+windows CI matrix *(MF2)*; checksum-mismatch,
  missing-asset, and missing-sidecar fail-closed cases; a malicious-archive matrix
  (`..`, absolute, symlink-escape) each rejected by `safe_extract` *(C4)*; a `#[cfg(not(feature =
  "self-update"))]` test pinning the compiled-out error string *(C3)*. Extend
  `tests/release_contract.rs` to (a) couple the module's target set to the `release.yml` matrix
  (subset check like the installers), (b) assert the new sidecar-count guard exists in `release.yml`
  *(MF1)*, and (c) assert `telex --json version --root <temp>` remains the stable invocation exposing
  every field `source_metadata()` reads *(MF3 argv-contract)*. `lite-task:release-upgrade:tests`

- [x] **WI6 — Docs.** Update the install/operating guide to document `telex upgrade`,
  `--version`, and `--from`; note fail-closed checksum behavior, supported platforms, and that
  `telex upgrade` **executes the downloaded (checksum-verified) binary** to read its metadata (so an
  OS-quarantine/Gatekeeper/SmartScreen failure has a recognizable support signature) *(MF3)*; note
  integrity-not-authenticity (signing deferred) *(premortem C9)*. Refresh `docs/guide/src/reference/
  cli.md` if generated and `docs/developing/releasing.md` for the new sidecar-count guard. Assemble
  `Docs.md` for the PR per paw-docs-guidance. `lite-task:release-upgrade:docs`

- [x] **WI7 — Release-workflow sidecar guard** *(MF1, in-scope hardening)*. Add a `release.yml`
  step before `publish` that fails if any archive lacks a sibling `.sha256` (sidecar count ==
  archive count), making the fail-closed premise machine-enforced. Coupled by a `release_contract.rs`
  assertion (WI5b). `lite-task:release-upgrade:release-guard`

## Open Questions

**None blocking.** OQ1 (TLS backend / feature gating / fail-closed stance) was resolved by a
gated council (HIGH confidence, genuine-sharper convergence): `native-tls` (KD2), default-on
`self-update` feature (KD1), fail-closed with no hatch (KD3), plus the KD7 hardening list.
Council artifacts:
`C:/Users/robemanuele/.copilot/session-state/42060612-7a76-4687-b58e-1ffe23a3c0f4/files/council-tls/synthesis.md`.

Non-blocking reopen conditions carried forward (not gating this plan; document + revisit if hit):

- A lean `sqlite`+`self-update`+musl/static *source* build fails to build/link `native-tls`
  reqwest → add a feature-scoped vendored-OpenSSL recipe or switch just that path to `rustls`.
  Treated as **not first-class for v1**; such users can compile the updater out.
- A real published release ever lacks a sidecar, or an enterprise env blocks sidecars while
  trusting the archive → revisit the fail-closed stance (loud env-var hatch).
- Evidence that the second rustls stack in default builds is negligible (or native-tls causes
  material pain) → revisit KD2.

## Planning Review Resolutions (SoT — general-reviewer, premortem + retrospective)

Society-of-thought planning review (claude-opus-4.7-high; parallel; interactive=false) raised 3
must-fix, 11 should-fix, 5 consider (across perspectives, deduplicated). Dispositions:

**Applied to plan** (prerequisite hardening / clarification — node outcome preserved):
- MF1 weak sidecar guarantee → WI7 + KD3 (release.yml sidecar-count guard, coupled in tests).
- MF2 Unix exec-bit loss → KD7(c)/WI2 (chmod 0o755 on staged binary) + WI5 (Linux CI exercises it).
- MF3 argv-contract coupling of `source_metadata` → WI5c (invocation-grammar contract test) + WI6
  (document the fork-before-install support signature). Manifest-sidecar-asset idea: **deferred**.
- SF tag normalization → KD5/WI2/WI4/WI5. `--repo` phishing/trust surface → KD6/WI3 (hidden + env).
  Token cross-host redirect leak → KD7(a). Single `safe_extract` + traversal matrix → KD7(b)/WI2/WI5.
  Real captured `/releases/latest` fixture → KD6/WI5. no-default-features/self-update CI combos +
  compiled-out error test → WI1/WI5.

**Deferred (out of scope; recommend orchestrator-route as follow-ups — carry to field report):**
- Nightly/periodic **real-GitHub canary** workflow (retrospective SF: no CI signal vs live GitHub).
- Local **freshness cache** to avoid an API hit per `telex upgrade` (retrospective SF-h).
- Shipping a **`manifest.json` release asset** so metadata is a fetch not a subprocess fork (MF3).
- **Cryptographic release signing** (authenticity beyond checksum integrity) (premortem C9).
- **Fuzz target** for `safe_extract` (retrospective C4).
- First-class **musl/static** `sqlite+self-update` distribution + vendored-OpenSSL recipe (council).

**Not adopted:** elevating a `--allow-unverified` escape hatch to first-class (council + review
agree fail-closed is correct; MF1's machine-enforced sidecar guard removes the stranding root cause).
Compile-time platform detection "can't learn new targets at runtime" (premortem C1) is inherent to a
compiled binary; documented, not changed.


