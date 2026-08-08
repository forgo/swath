# Security Policy

## Supported versions

Swath is pre-alpha; only the latest commit on `main` is supported. Once releases begin, this
table will list supported release lines.

| Version | Supported |
| ------- | --------- |
| `main`  | ✅        |

## Reporting a vulnerability

**Do not open a public issue for security problems.**

Email **elliott.richerson@gmail.com** with `[swath security]` in the subject. Include a
description, reproduction steps, and impact assessment if you have one.

(When this repository is public, GitHub Private Vulnerability Reporting will be enabled and the
"Report a vulnerability" button becomes the preferred channel — tracked in issue #50.)

**Response targets:** acknowledgment within **72 hours**; triage verdict within **7 days**;
coordinated disclosure within **90 days** of acknowledgment unless we agree otherwise.

## Security design

Deliberate security-relevant choices, recorded as they are made (the forgo-auth convention —
this section grows with the codebase):

- **Memory safety is structural:** the core is Rust with `unsafe_code = "warn"` enforced at the
  workspace level; `unsafe` requires justification and scopes the Miri schedule (ADR 0002,
  ENGINEERING.md §2).
- **Supply chain:** cargo-deny gates advisories/licenses/bans/sources on every PR and nightly
  (`security.yml` files issues on new advisories); all GitHub Actions are SHA-pinned; workflows
  are statically analyzed by zizmor in CI; workflow permissions are least-privilege
  (`contents: read` default, per-job elevation only).
- **No long-lived registry tokens:** publishing (when it begins) uses OIDC trusted publishing
  exclusively (ENGINEERING.md §5).
- **DCO sign-off** required on every commit (ADR 0003).
