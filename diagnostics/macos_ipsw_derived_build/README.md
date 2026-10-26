# Isolated derived ipsw build

Preparation only for issue #172. This route never invokes the resulting `ipsw`.
It needs its own exact-source BUILD review and freeze, permits attempt 1 only,
and changes no existing component pin or workflow.

The unchanged owner patch separates the three `sw_vers` stdout results from
private stderr. It does not establish the cause of the earlier failed run.
The resulting executable is derived tooling, not the published release.

Capacity follows dispositions 6419297367 and 6419617835: sequential, counted
HTTP-body prefetch into a frozen file proxy; a fixed non-sparse read-write
image; and an initial provisioned reserve before transfers. No process-socket
isolation, host-wide disk quota, live reserve floor or reproducibility is claimed.
The controller records known outside-volume writes and all failure evidence.
It detaches but never unlinks the backing image, on success or failure; runner
teardown is its lifetime limit. Persisted provenance is provisional: collection
requires a zero return after manifest finalization and a verified artifact set.
The binary remains subject to independent review, not pre-finish acceptance.
The first required-output failure remains the raised error and the primary
failure-receipt error; later sink failures are recorded separately. A failed
provenance write is never described as persisted evidence.

## Separate compile and final-binary file limits

Under [6428392720](https://github.com/lossyrob/telex/issues/172#issuecomment-6428392720),
the compile child alone requests RLIMIT_FSIZE soft=hard=`compile_file_bytes`,
2147483648 bytes (2 GiB), for each file it writes, including intermediates.
The ad-hoc-sign child still requests soft=hard=`binary_bytes`, 134217728 bytes
(128 MiB). Other child labels receive no new preexec limit. Each child command
record and the provenance `child_file_limit_configurations` list record both
configured values and the selected soft/hard request (or null). These are
configuration records, never kernel readback or nested-compiler attestation;
they also remain configuration if spawning or setting the limit fails.

Immediately after a successful compile, before any subsequent module checks,
binary hash, metadata, signing or export, `lstat` must establish a regular
produced `ipsw` no larger than the unchanged 128 MiB cap. Oversize, a nonregular
file or a failed stat stops the attempt without partial binary use. Later hash,
signature and export bounds remain in force. A failed compile does not inspect
or use a possible partial binary. Synthetic boundary tests mock stat sizes on
tiny files; they neither allocate large files nor measure real build outputs.

The existing fixed volume independently bounds aggregate logical capacity.
Outside-volume writes remain unclaimed; the larger per-file compile limit
does not establish a live reserve floor, successful compilation or capacity
sufficiency. An intermediate exceeding 2 GiB fails closed and returns to the
steward. No retry or further increase is authorized.

The steward classified the preceding reused 128 MiB compile limit as a
design-owned bound defect. Actual 40404832511 reported a file-too-large write
to an intermediate archive, but its partial size and kernel limit were not
observed. The correction is a policy decision, not a measured cause or a
capacity result; the public release binary's size is not a derived-size
prediction. No local compile, probe or size measurement accompanies it.

## Explicit empty-image recipe

Local correction under disposition 6424990314 changes only the image creation
option pair from `-format UDRW` to `-type UDIF`:

```text
/usr/bin/hdiutil create -size 12288m -fs APFS -volname TelexDerivedBuild -type UDIF {image}
```

Apple's [hdiutil manual, CREATE](https://github.com/keith/xcode-man-pages/blob/cb44759ff0c3f25182b2eeeb5d44ca0de9f3c01a/docs/hdiutil.1.html#L603-L660)
documents `-type` for empty read/write images and says explicit `UDIF` creates a
`UDRW` image of the specified size. `-format` is an image-from-source option,
not this empty-image recipe. The manual also documents `-size`, APFS via `-fs`
and `-volname`. No default format, source folder/device or additional switch is
used. This URL is a third-party HTML export of Apple's Xcode-bundled manual,
as described by the [mirror project](https://github.com/keith/xcode-man-pages);
it is not Apple-hosted documentation or attestation of the runner's tool.
That immutable export is Xcode 27.2 RC1 (27B48), not the pinned Xcode 26.6.
The Apple-hosted archived manual URL returned 404 in the owner's retained
audit. Native behavior matching the cited recipe remains a declared risk.

The fixed non-sparse read/write intent, 12 GiB logical capacity including
metadata, ENOSPC behavior, APFS/name, existing attach/detach and initial
provisioned reserve before fetch are unchanged. No post-failure backing stat
or other controller/finalization behavior is added. Synthetic argv tests are
not proof of a successful native image allocation.

Disposition [6425196426](https://github.com/lossyrob/telex/issues/172#issuecomment-6425196426)
clarifies that hdiutil's default OS-supplied, same-user attach mechanism is
within the trusted native tool boundary. Installed/privileged helpers, brokers
and new services remain excluded. No `-kernel`, alternate attach path or
fallback is added. An authorization prompt, escalation or attach failure fails
closed and returns to design; this controller is not an OS privilege monitor.

The exact plist `system-entities`/`mount-point` schema and APFS topology remain
unestablished by the retained manual. The strict codesign parsers are also
unchanged: `Signature=adhoc` must occur in stderr; any `Timestamp=` line must
equal `Timestamp=none` (a timestamp line is not required); legacy `:-`
entitlement extraction must leave stripped stdout empty. These spellings and
stream placement remain declared residual risks, not runner/source-attested
results. Unexpected parsed shapes fail closed with full bounded streams.
The existing static CodeDirectory checks remain; no optional parser refactor
or `-s -w` Mach-O enhancement is included. The retained iPhone-only
SecurityTool source is not macOS codesign CLI proof; the Homebrew Git banner
does not attest the controller's `/usr/bin/git` version.

## Pinned Go output contracts

The same disposition permits local correction of the owner's F1/F2 findings.
The source reference is Go 1.27.3 commit
`2aae1000ea1d4b28923b96db28acf4e3b99d6aff`, not a native build observation.

F1: the constructed child map still contains `GOENV=off`.
[cfg.EnvFile](https://github.com/golang/go/blob/2aae1000ea1d4b28923b96db28acf4e3b99d6aff/src/cmd/go/internal/cfg/cfg.go#L344-L350)
returns an empty environment-file path for that input, and
[MkEnv](https://github.com/golang/go/blob/2aae1000ea1d4b28923b96db28acf4e3b99d6aff/src/cmd/go/internal/envcmd/env.go#L79-L89)
emits the empty path as effective `GOENV`. The controller compares all 20
requested keys with an explicit expected map; only effective `GOENV` differs
from the child map. Every key is mandatory and exact, including empty-valued
keys. Missing `GOENV` fails. Provenance retains expected and observed maps on
success; the complete captured JSON stream remains available on mismatch.
No inherited environment is changed or forwarded.

F2: frozen `-trimpath`, `-ldflags` and both complete `-X` values stay unchanged.
[load/pkg.go](https://github.com/golang/go/blob/2aae1000ea1d4b28923b96db28acf4e3b99d6aff/src/cmd/go/internal/load/pkg.go#L2469-L2480)
omits `-ldflags` from build info under `-trimpath`. Metadata now requires that
setting to be absent, even if its value would be correct or empty. All other
expected settings remain exact.

Before and after explicit signing, the bounded Mach-O bytes must contain both
complete UTF-8 release values: `bounds.build.version` and `commit_string`,
not their symbol or assignment names. The existing file-hash cap is followed
by a bounded `maximum + 1` read, deadline/length/hash equality check and static
search in those same bytes. Each successful pre/post record retains the
matched value, UTF-8 length and first file offset with the file hash. The
compile command record retains the exact linker-flag argument. This proves
complete byte occurrence, not runtime variable lookup, uniqueness,
reproducibility or upstream binary equivalence. `ipsw` is never executed.

Guarded fixtures now model empty effective GOENV, absent metadata `-ldflags`
and tiny synthetic Mach-O files containing both complete values. The full
23-command fixture invokes the actual static parsers rather than stubbing
them. Missing/wrong effective keys, missing/shortened/wrong values, unexpected
metadata flags and post-sign corruption fail without export or acceptance.
These local checks do not constitute owner replay, whole-source approval or
native build proof; previous blocked source packets and receipts remain
immutable history.

## Public no-tags configuration and static import closure

Disposition [6426580271](https://github.com/lossyrob/telex/issues/172#issuecomment-6426580271)
removes only `-tags=sandbox` from the build arguments. The public frozen tree
does not contain the closed-source `pkg/sandbox` imports selected by that
tag. No missing source or module is added. Every other flag, F1/F2, patch,
toolchain, image recipe and numeric limit remains unchanged.

Metadata requires **absent `-tags`**, even an empty value.
[Pinned Go load/pkg.go:2492-2493](https://github.com/golang/go/blob/2aae1000ea1d4b28923b96db28acf4e3b99d6aff/src/cmd/go/internal/load/pkg.go#L2492-L2493)
appends this setting only when `cfg.BuildContext.BuildTags` is nonempty.
This is source reasoning, not a Go invocation.

The standard-library-only `static_imports.py` runs after exact source-tree
verification, before patching, compiler acquisition or module transfers.
It checks the frozen request plan against go.sum before using it. It records
`import-closure.json` under the existing evidence caps, then rejects any
unsatisfied check. Its source and tests are part of the explicit source closure.
Neither the checker nor its fixtures execute upstream source, `go list`, a
toolchain or the decoder.

The checker follows reachable main-module imports from `cmd/ipsw` for
darwin/arm64, cgo, gc, Go 1.1-1.27 and arm64.v8.0 with no custom tags.
It evaluates filename suffixes, modern Boolean `//go:build` and legacy
`// +build` expressions; modern constraints take precedence. Test/hidden files
are excluded. Package/import prefixes support comments, aliases, raw and
unescaped interpreted import strings. Unsupported escapes, malformed prefix
syntax, misplaced/unseparated constraints, experiment-tag uncertainty, nested
modules, vendor trees, module replacements/exclusions, missing/mixed packages,
unknown imports and main-module cycles fail closed. All loaded source bytes
must match the manifest's size and Git blob identity within existing source
byte/deadline limits. Reads use expected size plus one, not unbounded reads.

Each external import must match a module path in the frozen plan on a path
boundary and a required version in frozen go.mod; the longest matching module,
required version and all planned versions are recorded.
This is module membership, not proof that the package exists inside that
module or that MVS selects a particular version. Standard-library names are
classified by an explicit allowlist whose names were checked against source
directories in the hash-verified pinned Go archive. This is not target-specific
standard-library compilation proof. `C` is the cgo pseudo-import, not an
external module. Repeated import paths under aliases
collapse to one graph edge; import-name binding is not checked. The report
explicitly disclaims external transitive closure, compilation/type/link correctness, cgo libraries, assembly, embed
resources, generated code and runtime behavior. Unsupported standard-library
names fail rather than being silently treated as a planned dependency.

Tiny guarded fixtures include the historical sandbox-tagged configuration
failing on `github.com/blacktop/ipsw/pkg/sandbox`, no-tags selection of the
public stub, and a real controller-to-checker failure before compiler fetch.
The preparation replay also checked all 1665 frozen source identities before
using the owner-rehydrated inert bytes. Its no-tags graph contains 148 reachable
main-module packages, 717 selected Go files and 2335 import edges. The 119 unique
external imports map to 75 required/planned modules. The full historical
sandbox graph fails on missing `pkg/sandbox/graph`. These are parsed-source
results, not compilation proof. Source and Go archives were acquired once by
the owner under [6426649700](https://github.com/lossyrob/telex/issues/172#issuecomment-6426649700);
the helper did not fetch inputs. No upstream bytes are vendored or committed.

**Closed-source sandbox features are absent.** No release feature equivalence
or whole-binary equivalence is claimed. The owner must establish separately
that the frozen component commands and patched GetBuildInfo use do not depend
on sandbox-tagged code; a dependency returns to design, not source acquisition.
The owner's separate sealed purpose proof reports that `ipsw version` and
`ipsw ota patch rsr --cryptex` use untagged command registration and the
darwin/cgo GetBuildInfo/RawImagePatch route, not sandbox features. Root still
registers the public `sb` group. That selected-route proof does not replace the
whole-root graph check or claim a complete runtime call graph. Both exact
attributed receipts are included in the preparation packet before source review.

## Controller inputs and trusted imports

Under dispositions 6423437079 and 6423574173, the first statement of both `main`
and `execute` checks controller inputs. These six exact, case-sensitive names
are rejected on **presence**, including empty values:

| Name | Possible native use |
| --- | --- |
| `OPENSSL_CONF` | Config selection during trusted import initialization or later hashing/TLS |
| `OPENSSL_CONF_INCLUDE` | Native config include processing during initialization or later use |
| `OPENSSL_MODULES` | Native provider loading during initialization or later use |
| `SSLKEYLOGFILE` | Native SSL context creation/keylogging; `-I` is not proof of neutralization |
| `SSL_CERT_FILE` | Default certificate-file loading at TLS context setup |
| `SSL_CERT_DIR` | Default certificate-directory loading at TLS context setup |

The claim is **rejected before any controller action**, not before first native
use. Import-time native initialization by hashlib/_hashlib, ssl/http.client/
urllib and the unchanged a303 module may already have read OpenSSL config or
loader inputs. That is inside the trusted-interpreter/startup boundary. Imports
are not deferred; no startup-isolation launcher, CA bundle or verification
change is introduced. The source-only audit uses public CPython
`d220409a13b77de7a6c30abe16ea90db01528edc` and OpenSSL
`636dfadc70ce26f2473870570bfd9ec352806b1d`, not actual runner-library attestation.

Module scope is imports, path/import bootstrap, aliases, regex construction,
definitions and the contextmanager decorator. No controller network, child,
evidence hashing, SSL-context creation or file write runs there. An entry
rejection precedes bounds reads, evidence paths and gate work: only the
names-only exception is available; **no provenance or manifest exists**.

Allowed relevant inherited names are sorted into the existing provisional
provenance before the gate, with no values, hashes of values, lengths of values
or URLs. The inventory uses the existing 4 MiB per-file ceiling and fails on
overflow; aggregate artifact checks still apply. Categories are:

- `proxy`: every case-insensitive `_proxy` suffix, including `NO_PROXY` and mixed
  case. Record only, never reject. Both gate GET kinds and payload fetches use
  explicit `ProxyHandler({})`, so neither environment nor Darwin proxy discovery
  is used. No global opener, environment mutation or fallback is permitted.
- `go_compiler_sdk_search`: case-sensitive prefixes `GO`, `CGO_`, `CC`, `CXX`,
  `SDKROOT`, `DEVELOPER_DIR`, `TOOLCHAINS`, `CFLAGS`, `CPPFLAGS`, `CXXFLAGS`,
  `LDFLAGS`, `LIBRARY_PATH`, `CPATH`, `C_INCLUDE_PATH`, `CPLUS_INCLUDE_PATH`.
  These could affect tools if forwarded; their ambient values are excluded by
  construction, not exempted or adopted. `GOROOT_1_24_ARM64` is a name only; its
  value in the failed attempt remains unknown.
- `loader`: case-sensitive `DYLD` and `LD_` prefixes. Their startup effect is
  trusted infrastructure; recording is not loader neutralization. Child maps
  do not forward their values.
- `native_reference`: case-insensitive `OPENSSL`, `SSL_`, `OSSL_` prefixes and
  `SSLKEYLOGFILE`, `TSGET`, `RANDFILE`, `CTLOG_FILE`, `QLOGDIR`. The exact six
  rejected names above never pass. Other names are recorded, not newly rejected
  or claimed neutralized. Examples are `OPENSSL_ENGINES`, debug allocation
  `OPENSSL_MALLOC_FD`/`OPENSSL_MALLOC_FAILURES`, `OPENSSL_TRACE`, architecture
  `OPENSSL_ia32cap`/`OPENSSL_sparcv9cap`/`OPENSSL_ppccap`/`OPENSSL_armcap`/
  `OPENSSL_s390xcap`/`OPENSSL_riscvcap`; their use is native initialization or
  feature-specific under the trusted-library limit. `OPENSSL`, `OPENSSL_CONFIG`,
  `TSGET` are script inputs; `OPENSSL_WIN32_UTF8` is Windows-specific; `RANDFILE`,
  `CTLOG_FILE`, `QLOGDIR` and `OSSL_QFILTER` refer to RAND/CT/QUIC APIs not directly
  invoked here. Trusted native config can reference other names; this is not
  an exhaustive installed-library environment audit.
- `declared_control_plane`: `GH_TOKEN`, `GITHUB_EVENT_NAME`,
  `GITHUB_RUN_ATTEMPT`, `GITHUB_REPOSITORY`, `GITHUB_REF`,
  `GITHUB_WORKFLOW_REF`, `GITHUB_SHA`, `ImageVersion`. Existing authenticated
  gate/identity uses remain; only names enter this inventory. Existing explicit
  source/ImageVersion provenance is distinct. No token reaches a child map.

## Child and transport structure

All 23 planned invocations pass an explicit fresh dictionary through
`Runner.command` to `Popen`; `None` is rejected before launch. Initial control
uses the frozen build map with work-local HOME/TMPDIR (8 commands); volume build
uses the frozen volume-local map (14); detach has only PATH/HOME/TMPDIR (1).
Ambient Go/compiler/proxy/loader/native inputs are not merged into any map.
The effective `go env` equality check remains required.

The BUILD-local gate getter copies the pinned a303 bounded read logic, changing
only opener construction to add explicit empty proxies. It reuses the unchanged
refusing redirect handler and helpers. Canonical URL/authentication, redirect
refusal, deadline and byte caps remain; old diagnostic.py is not modified.
The proof includes both gate GET kinds, the payload transport, module/call-site
inventory, the whole 23-label sequence at mocked Popen, and minimal detach on
attach failure and execute failure. No claim is made that Go opens no sockets.

Synthetic tests use mocked processes, HTTP transports and filesystem metadata.
Do not run `build.py` locally. Review `bounds.json` and the exact source packet
before any hosted action. BUILDs 40369808285, 40382992497, 40392033737 and 40404832511 were consumed;
retries remain zero.
The independent actual verdict
[6425115159](https://github.com/lossyrob/telex/issues/172#issuecomment-6425115159)
is FINAL CHANGES-REQUIRED for 40382992497, with failure-data conservation
passing and no material field-report contradiction. It is not pending or
authority for this child. The later actual verdict
[6426534955](https://github.com/lossyrob/telex/issues/172#issuecomment-6426534955)
is also FINAL CHANGES-REQUIRED for failed compilation in 40392033737, with
conservation passing and no material field-report contradiction. Actual
[6428238479](https://github.com/lossyrob/telex/issues/172#issuecomment-6428238479)
is FINAL CHANGES-REQUIRED for failed compilation in 40404832511, with
conservation passing and no material field-report contradiction or omission.
It does not establish a measured limit, partial-file size or cause. Next is
whole owner replay/integration, then fresh
whole-source independent APPROVE, a NEW finite-attempt disposition and a NEW
genuine owner freeze. This local correction authorizes none. HOLD is unchanged.
