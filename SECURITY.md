# Security Policy

## Supported versions

Swath is pre-alpha; only the latest commit on `main` is supported. Once releases begin, this
table will list supported release lines.

| Version | Supported |
| ------- | --------- |
| `main`  | ✅        |

## Reporting a vulnerability

**Do not open a public issue for security problems.**

**Preferred:** use GitHub Private Vulnerability Reporting — the
["Report a vulnerability"](https://github.com/forgo/swath/security/advisories/new) button on the
repository's Security tab. It keeps the report, discussion, and any resulting advisory private
until coordinated disclosure.

**Fallback:** email **elliott.richerson@gmail.com** with `[swath security]` in the subject.

Either way, include a description, reproduction steps, and impact assessment if you have one.

**Response targets:** acknowledgment within **72 hours**; triage verdict within **7 days**;
coordinated disclosure within **90 days** of acknowledgment unless we agree otherwise.

## Security design

Deliberate security-relevant choices, recorded as they are made (the forgo-auth convention —
this section grows with the codebase):

- **Memory safety is structural:** the core is Rust with `unsafe_code = "warn"` enforced at the
  workspace level; `unsafe` requires justification and scopes the Miri schedule (ADR 0002,
  ENGINEERING.md §2).
- **Supply chain:** cargo-deny gates advisories/licenses/bans/sources on every PR and nightly
  (`security.yml` files issues on new advisories); `dependency-review-action` fails PRs that
  introduce vulnerable dependencies; all GitHub Actions are SHA-pinned; workflows are statically
  analyzed by zizmor in CI (findings fail CI and upload to code scanning); workflow permissions
  are least-privilege (`contents: read` default, per-job elevation only).
- **Repository surfaces:** secret scanning with push protection is active; OpenSSF Scorecard and
  CodeQL (Rust) run on schedule and publish to code scanning (ENGINEERING.md §5, issue #50).
- **No long-lived registry tokens:** publishing (when it begins) uses OIDC trusted publishing
  exclusively (ENGINEERING.md §5).
- **DCO sign-off** required on every commit (ADR 0003).
