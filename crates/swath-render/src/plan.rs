// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The single [`RenderPlan`] constructor (issue #95).
//!
//! Every plan a layer serves — operator config (static and catalog mode),
//! the built-in fixture registry, and the openEO process compiler — is one
//! of two shapes: an RGB **composite** or a colormapped **band-math**
//! expression, each optionally rescaled. [`PlanSpec`] names that shape
//! once, and [`plan_for`] is the only place the shape becomes an
//! executable [`RenderPlan`]: op order, input derivation, and the
//! persisted-metadata mirror all live here, so the executable plan and the
//! catalog's [`PlanKind`]/[`Rescale`]/[`Colormap`](DomainColormap) record
//! cannot drift apart. The `plan_roundtrip` proptest pins the agreement:
//! a spec authored as a standard openEO graph compiles back to exactly
//! the plan this constructor builds.
//!
//! Placement: the constructor sits beside the IR it targets. Both
//! `swath-cli` (config) and `swath-api` (registry, openEO services)
//! already depend on this crate, and the storage vocabulary flows the
//! right way — this crate depends on `swath-core`, never the reverse.

use swath_core::catalog::{Colormap as DomainColormap, PlanKind, Rescale};

use crate::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};

/// A plan kind with its band bindings: everything [`plan_for`] needs to
/// build the executable plan and its persisted mirror.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PlanSpec {
    /// Three named bands as the R, G, B planes.
    Composite {
        /// Band for the red channel.
        r: String,
        /// Band for the green channel.
        g: String,
        /// Band for the blue channel.
        b: String,
        /// Linear rescale onto 0..=255 after composition; `None` = raw
        /// values clamp at quantization (the identity 0..255 mapping).
        rescale: Option<(f64, f64)>,
    },
    /// A band-math expression producing gray planes, then colormapped.
    BandMath {
        /// The per-pixel expression.
        expr: Expr,
        /// Linear rescale onto 0..=255 after the math; `None` = raw
        /// values clamp at quantization (the identity 0..255 mapping).
        rescale: Option<(f64, f64)>,
        /// The palette applied to the gray result ([`Colormap::Grayscale`]
        /// is the identity).
        colormap: Colormap,
    },
}

/// The persisted catalog vocabulary mirroring a spec's plan — the
/// `swath:layers` fields describing exactly what the plan renders.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PlanMetadata {
    /// How dataset bands become pixels ([`PlanKind`]).
    pub kind: PlanKind,
    /// The value range mapped onto 0..=255. A spec without a rescale
    /// renders the identity mapping of the 8-bit range, so this is
    /// `0..255` — the persisted record and the absent `Rescale` op
    /// describe the same rendering.
    pub rescale: Rescale,
    /// The persisted colormap: every band-math plan carries one
    /// (grayscale included); a composite renders RGB directly and
    /// carries none.
    pub colormap: Option<DomainColormap>,
}

/// The NDVI expression over the two named bands:
/// `(nir - red) / (nir + red)` — the shape shared by the config `ndvi`
/// kind, the fixture registry, and the openEO `ndvi` process.
#[must_use]
pub fn ndvi_expr(nir: impl Into<String>, red: impl Into<String>) -> Expr {
    let (nir, red) = (nir.into(), red.into());
    (Expr::band(nir.clone()) - Expr::band(red.clone())) / (Expr::band(nir) + Expr::band(red))
}

/// Every band a spec's ops reference, first-reference order,
/// deduplicated — the plan's inputs, so serving fetches nothing the
/// plan does not read.
fn referenced_bands(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Band(name) => {
            if !out.iter().any(|n| n == name) {
                out.push(name.clone());
            }
        }
        Expr::Const(_) => {}
        Expr::Binary { lhs, rhs, .. } => {
            referenced_bands(lhs, out);
            referenced_bands(rhs, out);
        }
    }
}

/// The persisted spelling of an IR colormap, variant for variant.
fn domain_colormap(map: Colormap) -> DomainColormap {
    match map {
        Colormap::Grayscale => DomainColormap::Grayscale,
        Colormap::Viridis => DomainColormap::Viridis,
        Colormap::Magma => DomainColormap::Magma,
        Colormap::RdYlGn => DomainColormap::RdYlGn,
    }
}

/// **The** `RenderPlan` constructor: lowers a [`PlanSpec`] into the
/// executable plan and, in the same motion, the persisted
/// [`PlanMetadata`] mirroring it. Inputs are derived from the ops
/// (first-reference order, deduplicated); output is always PNG.
#[must_use]
pub fn plan_for(spec: &PlanSpec) -> (RenderPlan, PlanMetadata) {
    let (bands, ops, kind, colormap) = match spec {
        PlanSpec::Composite { r, g, b, rescale } => {
            let mut bands = Vec::new();
            for band in [r, g, b] {
                if !bands.iter().any(|n| n == band) {
                    bands.push(band.clone());
                }
            }
            let mut ops = vec![PixelOp::Composite {
                r: r.clone(),
                g: g.clone(),
                b: b.clone(),
            }];
            if let Some((min, max)) = *rescale {
                ops.push(PixelOp::Rescale { min, max });
            }
            let kind = PlanKind::Composite {
                r: r.clone(),
                g: g.clone(),
                b: b.clone(),
            };
            (bands, ops, kind, None)
        }
        PlanSpec::BandMath {
            expr,
            rescale,
            colormap,
        } => {
            let mut bands = Vec::new();
            referenced_bands(expr, &mut bands);
            let mut ops = vec![PixelOp::BandMath(expr.clone())];
            if let Some((min, max)) = *rescale {
                ops.push(PixelOp::Rescale { min, max });
            }
            ops.push(PixelOp::Colormap(*colormap));
            let kind = PlanKind::BandMath {
                expression: expr.to_string(),
            };
            (bands, ops, kind, Some(domain_colormap(*colormap)))
        }
    };
    let (min, max) = match spec {
        PlanSpec::Composite { rescale, .. } | PlanSpec::BandMath { rescale, .. } => {
            rescale.unwrap_or((0.0, 255.0))
        }
    };
    let plan = RenderPlan::new(
        bands.iter().map(BandInput::new).collect(),
        ops,
        OutputSpec::new(TileFormat::Png),
    );
    (
        plan,
        PlanMetadata {
            kind,
            rescale: Rescale { min, max },
            colormap,
        },
    )
}

#[cfg(test)]
mod tests {
    use swath_core::catalog::{Colormap as DomainColormap, PlanKind, Rescale};

    use super::{PlanSpec, ndvi_expr, plan_for};
    use crate::ir::{Colormap, Expr, PixelOp, TileFormat};

    #[test]
    fn composite_spec_builds_ops_inputs_and_metadata_together() {
        let (plan, meta) = plan_for(&PlanSpec::Composite {
            r: "b04".into(),
            g: "b03".into(),
            b: "b02".into(),
            rescale: Some((0.0, 3000.0)),
        });
        let inputs: Vec<&str> = plan.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(inputs, ["b04", "b03", "b02"]);
        assert_eq!(plan.ops.len(), 2);
        assert_eq!(
            plan.ops[1],
            PixelOp::Rescale {
                min: 0.0,
                max: 3000.0
            }
        );
        assert_eq!(plan.output.format, TileFormat::Png);
        assert!(matches!(meta.kind, PlanKind::Composite { .. }));
        assert_eq!(
            meta.rescale,
            Rescale {
                min: 0.0,
                max: 3000.0
            }
        );
        assert_eq!(meta.colormap, None, "composites render RGB directly");
    }

    #[test]
    fn band_math_spec_colormaps_and_mirrors_the_expression_text() {
        let (plan, meta) = plan_for(&PlanSpec::BandMath {
            expr: ndvi_expr("b8a", "b04"),
            rescale: Some((-1.0, 1.0)),
            colormap: Colormap::RdYlGn,
        });
        let inputs: Vec<&str> = plan.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(inputs, ["b8a", "b04"], "first-reference order, deduped");
        assert_eq!(plan.ops.last(), Some(&PixelOp::Colormap(Colormap::RdYlGn)));
        assert_eq!(
            meta.kind,
            PlanKind::BandMath {
                expression: "(b8a - b04) / (b8a + b04)".to_owned()
            }
        );
        assert_eq!(meta.colormap, Some(DomainColormap::RdYlGn));
    }

    #[test]
    fn absent_rescale_omits_the_op_and_persists_the_identity_range() {
        let (plan, meta) = plan_for(&PlanSpec::BandMath {
            expr: Expr::band("b"),
            rescale: None,
            colormap: Colormap::Grayscale,
        });
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, PixelOp::Rescale { .. })),
            "no rescale op when the spec has none"
        );
        assert_eq!(
            meta.rescale,
            Rescale {
                min: 0.0,
                max: 255.0
            },
            "the persisted record spells out the identity mapping"
        );
    }
}
