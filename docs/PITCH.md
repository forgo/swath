# Swath — Positioning & Pitch

*Maintainer-facing pitch material, split out of `CHARTER.md` (its former §11-12) so the
charter stays a contributor document — vision, principles, phases. This document is outside
the contributor reading path (`README.md` "Documentation"); nothing in the build depends on it.*

---

## 1. Positioning & monetization

**Open-core.** A permissive, genuinely useful, self-hostable core (the platform, the engine, the control
plane, the x-ray tooling) that earns adoption and stars on its own. A commercial layer on top for teams
who don't want to run it themselves or need enterprise/government features:

- **Managed / hosted Swath** (the single pane + materialization SLAs, run for you).
- **Enterprise features:** SSO/SAML, fine-grained RBAC, audit, multi-tenant governance.
- **Government readiness:** compliance posture (FedRAMP-style controls), air-gapped/on-prem support.
- **Premium connectors & support:** priority data-source integrations, SLAs, professional services.

This mirrors proven models in the space — Development Seed's services around eoAPI, and Earthmover's
Arraylake around open-source Icechunk — but Swath's commercial wedge is distinct: the **managed single
pane + the materialization guarantees**, which is precisely the part that is hardest to operate well.

## 2. How this complements the founder conversation

This charter is framed as an independent open-source project, but it is deliberately the productized
platform layer that an EO-products government contractor most needs. Being the maintainer of the OSS core
makes its author the technical center of gravity for exactly that problem, and because Swath layers onto
an existing pgstac/TiTiler deployment it's adoptable there with near-zero migration cost. It exceeds the
"just orchestrate what exists" framing by owning what that framing misses: a **standards-native**
(openEO/OGC), **embedding-aware** loop, plus the cost-aware materialization brain that turns "assemble
the tilers" into an actual product.
