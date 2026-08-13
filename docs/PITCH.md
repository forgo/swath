# Swath — Positioning & Pitch

*Maintainer-facing pitch material, split out of `CHARTER.md` (its former §11-12); outside the
contributor reading path — nothing in the build depends on it.*

---

## 1. Positioning & monetization

**Open-core.** A permissive, genuinely useful, self-hostable core that earns adoption on its
own; a commercial layer on top: managed/hosted Swath (the single pane + materialization SLAs),
enterprise features (SSO/SAML, RBAC, audit, multi-tenancy), government readiness, premium
connectors and support. This mirrors proven models (Development Seed around eoAPI, Earthmover's
Arraylake around Icechunk), but Swath's commercial wedge is distinct: the **managed single pane
+ the materialization guarantees** — precisely the part hardest to operate well.

## 2. How this complements the founder conversation

Framed as an independent open-source project, Swath is deliberately the productized platform layer an
EO-products government contractor most needs; maintaining the OSS core makes its author the technical
center of gravity for that problem, and layering onto an existing pgstac/TiTiler deployment means
near-zero migration cost. It exceeds the "just orchestrate what exists" framing by owning what that
framing misses: a **standards-native**, **embedding-aware** loop plus the cost-aware materialization
brain that turns "assemble the tilers" into an actual product.
