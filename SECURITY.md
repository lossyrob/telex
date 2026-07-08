# Security Policy

## Supported versions

Telex is distributed as a single, batteries-included binary published to
[GitHub Releases](https://github.com/lossyrob/telex/releases). Security fixes land
on the latest release. Rather than maintain a version table that drifts, the policy
is simple:

- The **latest published release** receives security fixes.
- If you are on an older release, upgrade to the latest before reporting, and
  confirm the issue still reproduces there.

Telex is pre-1.0 (`0.x`); interfaces and internal behavior may change between
minor releases.

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues,
pull requests, or discussions.**

Report privately through GitHub's private vulnerability reporting:

1. Go to <https://github.com/lossyrob/telex/security/advisories/new>, or open the
   repository **Security** tab and choose **Report a vulnerability**.
2. Describe the issue, the affected version (`telex --version`), the platform, and
   the steps to reproduce. A minimal proof of concept is very helpful.

If GitHub private reporting is unavailable to you, open a public issue that asks
for a private contact channel **without** including any vulnerability details, and
a maintainer will follow up.

## What to expect

- **Acknowledgement:** within 7 days of your report.
- **Assessment:** we will confirm the issue, determine its severity, and keep you
  updated on remediation progress.
- **Disclosure:** we prefer coordinated disclosure. Once a fix is released, we will
  credit reporters who wish to be named.

## Scope

Telex serves a single operating-system user over local IPC by default; its trust
model, where data is stored, and how secrets are referenced (never written to
config) are documented in the
[Security and data](https://lossyrob.github.io/telex/concepts/security.html) guide.
Reports that depend on an already-compromised local user account, or on placing the
store or socket where other OS users can read it (explicitly called out as
out-of-model in that guide), are generally out of scope. When in doubt, report it
and let us assess.
