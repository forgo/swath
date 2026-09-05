<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# ADR 0031 — Credentialed sources and server-side egress wait for OIDC/RBAC

**Status:** Accepted · **Date:** 2026-09-05 · **Amends:** ADR 0030 (the sources domain) ·
**Refs:** `docs/ROADMAP.md` §3 item 5, `crates/swath-api/src/sources.rs`,
`crates/swath-cli/src/serve.rs`, issue #421

## Context

ADR 0030 decided the sources domain in one go and said its waves would land in order. Wave A shipped
a **read-only** resource, and the reason was left implicit in the module docs. It should not be
implicit: it is a security dependency, and an unrecorded dependency is one a later PR can lift by
accident.

Today the API has no notion of who is asking. `swath serve` binds a port and answers whoever reaches
it; there is no identity, no role and no audit trail. A route that *creates* a source would
therefore let anyone with reach to that port:

- point the server at a host of their choosing (an SSRF engine, ADR 0030 §5), and
- bind a credential profile the operator provisioned, spending the operator's access.

Both of those are exactly what Wave B (#423, credentials by reference) and Wave C (#419, the STAC
client) are about. So the dependency runs one way and must be recorded in both places.

## Decision

1. **The sources API stays read-only until Charter Phase 3's OIDC/RBAC lands** (ROADMAP §3 item 5).
   `GET /sources` and `GET /sources/{sourceId}` are the whole HTTP surface; sources are declared in
   the deployment's configuration file, which is already an operator-only channel.

2. **The mutating routes are ABSENT, not forbidden.** There is no handler to authorise — the same
   posture ADR 0016 took for read-only serving. That is a stronger guarantee than a 403, because it
   cannot be defeated by a middleware misconfiguration, and it is asserted by a test rather than
   reviewed.

3. **The lifting condition is written down.** All three must exist before a source can be created
   over HTTP:

   1. **Authentication** — the server knows who is asking (OIDC).
   2. **Authorisation** — a role that may *manage* sources, distinct from one that may *read* them
      (RBAC).
   3. **An audit trail** — source creation, credential-profile binding and egress consent are each
      recorded with the identity that performed them.

   Wave B and Wave C may build their domain and their adapters before that; what they may not do is
   expose a route that creates or edits a source, or one that causes an outbound fetch on an
   unauthenticated caller's say-so.

## Consequences

- A later PR that adds `POST /sources` fails a test that names this ADR, so the interlock cannot be
  lifted silently.
- Operators can still manage sources today: the config file is the channel, and a restart is the
  apply step (`docs/CONFIG.md`).
- Wave C's egress allowlist is necessary but **not sufficient**: an allowlisted host reached on an
  anonymous caller's request is still an anonymous caller directing the server.

## Alternatives considered

**Ship the mutating routes behind a 403 until auth lands.** A route that exists is a route that can
be exposed by a configuration mistake; absence cannot be misconfigured.

**Ship them behind a shared bearer token.** That is a long-lived credential, which ENGINEERING §5
rules out, and it answers "who" with "someone who has the token" — which is not an audit trail.
