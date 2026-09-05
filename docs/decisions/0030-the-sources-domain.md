<!-- SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
     SPDX-License-Identifier: Apache-2.0 -->

# ADR 0030 — The sources domain: origins, credentials by reference, allowlisted egress

**Status:** Accepted · **Date:** 2026-09-04 ·
**Refs:** `crates/swath-core/src/sources.rs`, `docs/ENGINEERING.md` §5,
`crates/swath-api/src/datasets.rs`, issues #422, #415–#421, #423, #424

## Context

Nothing in the tree knows where data came from. Ingest is one watched directory; a dataset knows
where its bytes are, and no object records the origin that produced them or whether that origin is
healthy. A Sources screen would have nothing to render, and a line like "watching · last event 4 min
ago" would have to be invented rather than measured.

Sources also open a security surface this project has not had: a credential, and an outbound fetch.
`datasets.rs` records today's posture — the server never fetches remote metadata, so there is no
SSRF surface — and federation would change that. A new surface deserves its own dated decision
rather than a paragraph appended to an existing one, so the whole shape is decided here even though
it lands in waves: the later issues implement this decision rather than make one.

## Decision

### 1. A `Source` is an origin, not a transport

A source has an id, a **kind** (what sort of origin it is), a **target** (where it points, in that
kind's own words), the **bindings** naming which datasets it feeds, and its **state**. It does not
own bytes, does not own granules, and is never in the read path of a tile. Deleting a source removes
the origin, not the data it produced.

### 2. State is derived from recorded events, never stored as a field

`SourceState` is a function of the source's event history — the last event, the last error, the
counts. Nothing sets a status field, because a field someone forgot to update is exactly how "healthy"
becomes a lie. A source with no events yet is `Unknown`, and the UI renders an em dash rather than
inventing a reassuring default.

### 3. Deleting a source is explicit about the data

Deletion removes the source and its event history. **The granules it ingested stay**, and they stay
attributed: each carries the source id it arrived under, so a deleted origin leaves traceable data
rather than orphans. Deleting the data is a separate, named act on the datasets themselves.

### 4. Credentials by reference — Swath stores the name, never the secret (Wave B)

A source may name a **credential profile the operator provisions in the environment or the instance
role**. Swath persists that *name* and reports only whether it resolves. No secret value is ever
written to the catalog, the object store, a log line, a trace envelope, or any API response. This is
what makes `401 · credentials expired` a measured fact rather than an invented one, keeps
ENGINEERING §5's "no long-lived tokens" posture intact, and adds no secret store to operate.

### 5. Egress is allowlisted, operator-initiated, and bounded (Wave C)

A server-side fetch happens only when an operator asks for one, only to a host on an explicit
allowlist, never following a redirect off that host, and under size and time caps. The `datasets.rs`
no-fetch note is amended to say exactly this when Wave C lands: the no-SSRF property is then
preserved by policy rather than by absence, and the policy is the allowlist.

### 6. Requester-pays is a consent step that reports bytes, not dollars

A source whose target charges the reader requires an explicit proof step before it is used, and that
step reports the bytes and requests it actually made. It never states a dollar figure — nothing in
the system can assert one.

## Consequences

- The Sources screen renders measured state or an em dash; there is no third option.
- A source's health cannot drift from reality, because it is not stored.
- Wave B ships no secret store, and the "no secret reaches the catalog" invariant is a test rather
  than a convention.
- Wave C's blast radius is the allowlist. An empty allowlist is a working configuration: it means
  the server fetches nothing, which is exactly today's behaviour.
- Anything credentialed stays behind the Phase 3 OIDC/RBAC work (#421), as an explicit dependency.

## Alternatives considered

**A status field on the source.** Simpler to read and impossible to keep true; rejected by the
project's standing rule that the UI never states what it did not measure.

**A secret store inside Swath.** Encryption at rest, rotation, an audit trail, and a new attack
surface — all to hold something the deployment's own environment already holds. Rejected.

**Fetching remote STAC on any user action.** That is an SSRF engine. The allowlist plus
operator-initiation is the narrowest thing that still federates.
