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
before any hosted action. One BUILD attempt was consumed; retries remain zero.
Any future hosted work needs a fresh whole-source APPROVE, a NEW finite-attempt
disposition and a NEW owner freeze. This local correction authorizes none.
