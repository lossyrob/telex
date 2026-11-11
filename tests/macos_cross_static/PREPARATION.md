# R2 local preparation: not launch authority

This source implements only the local preparation authorized by
[6804129659](https://github.com/lossyrob/telex/issues/172#issuecomment-6804129659),
with the current-branch amendment
[6804250104](https://github.com/lossyrob/telex/issues/172#issuecomment-6804250104).
No decoder host, label or target release is selected. `host_first_policy.json`
is deliberately unbound. The existing runner label and bounds are historical
consumed-source inputs, not a new selection. The prepared workflow stops before
host observation or component work while that policy is unbound.

No push of any kind is authorized. The local first commit descends directly
from `3a0d88d0320507660096a1e0662b2d770fe9d37d`; the remote proof ref stays at
3a. A later steward ruling must name the push target, by default a new remote
ref, never the existing proof ref. The current workflow/gate branch remains
historical, so future retargeting is itself a reviewed source change.

## Documentation reader, without label selection

`host_first.documentation_gate` takes supplied local bytes and metadata, not
URLs to fetch. `host_first.py --docs ABS_POLICY ABS_README ABS_INDEX
ABS_ACQUISITION ABS_OUTPUT` is its offline pre-dispatch interface. It checks:

- one pinned runner-images revision and matching acquisition revision/path/time;
- exact SHA-256s of the image README and root label index;
- the chosen label's unique index row, architecture and linked README;
- the expected product/build and a caller-declared positive freshness bound.

The acquisition record has `revision`, `readme_path`, `readme_sha256`,
`index_sha256`, and `observed_at`. The reviewed policy's `documentation` has
those same keys plus `maximum_age_seconds`. They must agree. Raw bytes are
authenticated to the pins; these fields do not independently prove who
retrieved them. A later reviewer/owner must authenticate the pinned public
revision. R2 supplies no filled acquisition record or time limit.

Future policy `documents` contains the exact `readme_utf8`, `index_utf8` and
`acquisition` fields, so all input bytes participate in the reviewed source
closure. No unmanifested documentation file is loaded by the host step.

The binding schema is exactly `label`, `architecture`, `product`, `build`,
`kern_osversion`, `kern_uuid`. Canonical uppercase UUID, matching build and
kern.osversion, and explicit architecture are required. No arbitrary defaults,
major-version tolerance or fallback label is allowed.

A contradiction, absent field, stale record, wrong revision/label/architecture
or ambiguous metadata rejects before dispatch eligibility. A match is only
`documentation-consistent; live allocation unverified`, with
`authorizes_launch=false`. The owner must use a fresh docs gate before each
future separately authorized action; the workflow rechecks those same pins
and freshness. The reader does not push, dispatch, reserve a host, or grant
authority.

## Same-job host-first observation

The first attempt step after checkout and guarded offline tests is
`Host-first observation`. Its only external commands are absolute `uname -m`,
`sw_vers -productVersion`, `sw_vers -buildVersion`, `sysctl -n kern.osversion`,
`sysctl -n kern.uuid`, and `git -C <source> rev-parse HEAD`. All are bounded
identity observations. It records full stdout/stderr/return values, architecture,
build/product/kernel identity and source SHA, run ID/attempt/job, and ImageOS/
ImageVersion as **context**, not security attestation.

The same-job step precedes candidate/artifact acquisition, Apple transfer,
decoder or tool staging, native resolution and product steps. The component
requires accepted host evidence for this exact run/source and verifies its
manifest, policy and workflow bytes before constructing a controller or doing
authority API reads. The existing in-component host check remains defense in
depth; it is not replaced by a docs match. NativeBinding construction occurs
only after host acceptance.

`host-first-observation-v1` records either `host-accepted` or
`host-first-refusal`. Missing/malformed command results, unbound preparation,
wrong attempt or invalid source produce a separate preparation/observation
failure, **not** a proven non-consuming HFR. A refusal does not authorize
launch or independently decide attempt consumption.

The six-member evidence bundle contains the observation, exact workflow/policy,
README/index and acquisition metadata, plus its hash/size manifest.
`verify_evidence` recomputes those pins, the documentation gate, exact host
command/field consistency and matching/refusal status. An exact reviewed
pre-host step grammar rejects any additional action, command, condition or
environment mutation. The zero-effect claim additionally requires an
independently obtained job step result list: checkout and offline tests
successful, host failed, component skipped. The owner maps the complete
provider step list to these logical steps and must reject omissions or any
extra consuming step; self-supplied four names alone are not authenticated
provider evidence.

`classify_refusal` is a data-only **proposal** function: mandatory manifest and
step checks precede any count. The caller must independently authenticate the
future finite, source/closure and complete job result. Source review verifies
that the pre-host script and guarded tests cannot reach consuming paths; a
zero counter alone is insufficient. This is a non-adversarial provider/source
model, not an attestation of an adversarial host.

Host evidence is bounded within the existing per-job 600-file/32MiB budget,
including the component evidence. No native snapshot/domain/file/time cap is
increased. Artifact upload of diagnostic output is not artifact acquisition.
No retry, re-dispatch, matrix fan or second host job is added.

## Future finite text template (INERT)

The following is only a specification for a later steward-authenticated
finite. It contains no live authority marker, usable source identity or
`allow_launch` value. Unbound/template values are rejected by the data reader.

> Future finite: [NEW comment URL and complete body hash].
> Bind [new reviewed source/closure/bounds], [later named remote ref/workflow],
> jobs exactly Intel25, [new genuine owner freeze], unchanged target inputs and
> candidate identity/retention. One consuming attempt; zero retries.
> Explicit HFR cap: [1 or 2], never above 2, per this finite.
> HFR is effective only for this future finite and exact source.
> Every dispatch is a separate owner action; no automatic continuation.
> Refusal is non-consuming only after complete same-job host evidence,
> authenticated provider step results and zero forbidden effects are verified.
> Any consuming effect before refusal consumes the attempt.
> Reaching the HFR cap ends this finite; return to the steward.
> No old finite or consumed attempt is restored. In particular 6664239510
> remains consumed.

The proposed data fields are `schema=future-finite-hfr-v1`,
`finite_comment_id`, `finite_body_sha256`, `source_sha`, `hfr_cap`,
`one_consuming_attempt=1`, `retries=0`, `every_dispatch_owner_directed=true`,
and `effective=true`. Only an authenticated future finite can supply them;
the literal fields are not authentication or permission. Historical a06/3d/3a
sources and pre-HFR finite IDs are rejected. The classifier returns
`finite_ended=true` on the cap and never returns automatic dispatch authority.

**Conservation:** existing review -> finite -> genuine freeze exact-body/account/
time/source bindings, first-attempt condition, Intel-only job set, original
stops and one-push intent remain unchanged. The live gate still refuses a
second same-source run and recognizes no HFR terms. R2 does not silently exempt
prior runs or implement a new dispatch path. A later reviewed, steward-ruled
launch change must explicitly reconcile future HFR history with that guard
before any second owner action can be eligible. This is intentionally
preparation, not an effective finite or launch-ready source.

## S2: effect records, not transport inference

`push_effect_record` is a pure record constructor, never a launcher. It records
`command_target_argument`, observed remote ref before/after, run head/event/
workflow/Intel25-only job set/count/attempt/actor. Effective transport and Git
credential principal remain `not attested`. No resolver, credential lookup,
config inspection, authentication change or push is present.

The consumed launch programs and their `actual_target_used`/stdout discrepancy
remain untouched in retained evidence. Their disposition under6665465986 stays
a closed precision limit, not a finite breach.

## S6: coherent rebind review surface (UNSELECTED)

Future source review must cover these as one decoder-host binding:

| Surface | Required reviewed change or evidence |
|---|---|
| Runner/docs/HFR | Chosen label, architecture, build/product, kern.osversion/UUID, pinned docs, same-job observation and manifest |
| Tools and SDK | Exact hdiutil/xcrun and otool C/D resolution, developer/toolchain, canonical/versioned SDK A/B and metadata identities |
| Controller/cache/library | Actual controller-loaded library, backing cache/subcache and raw Mach-O identities, paths/hashes and environment |
| Child use | Exact pinned native decoder invocation, effective architecture/loader environment, exact-child library trace, postdecoder recheck before semantic use |
| Final checks | Native/tool/candidate rehash, expiry and unchanged bounded cleanup |

Target releases are a separate operator decision; decoder-host method belongs
to the steward. Target URLs, members, all ancestry/hash/family rules, all-present
x86_64/x86_64h analysis, shared budgets and ARM24 semantics are unchanged.
No concrete future label/build/KDK or wrapper identity is supplied by R2.

## S9: operative supersession index

This index annotates historical text; it does not edit accepted packets or the
owner's plan.

| Historical surface | Operative disposition |
|---|---|
| One-shot signal completion | Rejected810bd24f; selected plan is fresh exact WNOWAIT plus unsaturated leader-only two-int census, seal, exact reap, ESRCH |
| Original two-job component and one infrastructure retry | Superseded by Intel-only6468291693; five BUILDs and a06/3d/3a consumed, retries0 |
| 12-file3a source closure and source review | Historical exact3a only. R2 has an expanded closure; no review/finite/freeze transfers |
| ARM optional244 residual and "five roots" | Closed scoped6665904411, count corrected6665937979: seven roots; physical/product acceptance does not follow |
| 6664239510/6664472672 future launch language | Consumed42074395136; no repair/retry/push authority |
| R1's unresolved positive-host method | Superseded only by6804129659's future-finite same-job HFR terms; not retroactive |
| New-local-branch wording | Replaced by6804250104: current local branch, direct3a parent, no push, later default new remote ref |
| Product S3/S4/S7/S8 | Specifications remain unapplied; no Rust/product implementation, receipt or restart credit |
| Target scope/admission | Operator A/B/C is separate; default existing exact targets until decision. Shipping macOS none, HOLD/items2-5 open |

## Offline validation and next gate

`offline_tests.py` installs process/network/native-loader guards **before**
discovering or importing tests. Fixtures may create synthetic in-memory
controllers and private temporary files, not a real native binding or tool
stage. Native constructors/resolution/decoder/download paths are mocked in
ordering tests; attempts to use real OS process, network or native-library
effects fail the runner. No supplied historical launcher/checker is replayed.

Run local source tests through that guarded entrypoint. A fresh **nonauthor**
independent source review, then steward ruling, is required before any push.
Any future attempt separately requires a new finite, genuine owner freeze,
explicit HFR terms, selected coherent host binding and named push target.
