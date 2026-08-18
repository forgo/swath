// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The manifest schema v1 contract: the persisted JSON shape is pinned by
//! snapshot (any change must show up as a reviewed snapshot diff — schema
//! changes bump `manifest_version` instead), and text → domain → text is the
//! identity for a representative manifest exercising every field.

use swath_manifest::{
    ChunkRef, GeoTransform, Georef, GeorefCrs, ManifestVersion, VirtualArray, VirtualManifest,
};

/// A VNP09GA-shaped representative: one sinusoidal (proj4) georeferenced
/// chunked array, one EPSG-coded array, one bare metadata array.
fn representative() -> VirtualManifest {
    VirtualManifest {
        manifest_version: ManifestVersion,
        generator: "swath-referencer".to_owned(),
        source: "VNP09GA.A2012019.h33v12.002.2023122182434.h5".to_owned(),
        arrays: vec![
            VirtualArray {
                name: "HDFEOS/GRIDS/VIIRS_Grid_1km_2D/Data Fields/SurfReflect_M1_1".to_owned(),
                shape: vec![1200, 1200],
                chunks: vec![600, 600],
                dtype: "int16".to_owned(),
                codecs: vec!["zlib:8".to_owned()],
                georef: Some(Georef {
                    crs: GeorefCrs::Proj4("+proj=sinu +R=6371007.181 +units=m +no_defs".to_owned()),
                    transform: GeoTransform::north_up(
                        16_679_257.796,
                        -3_335_851.559,
                        926.625_433_055_833_3,
                        -926.625_433_055_833_3,
                    ),
                    nodata: Some(-28_672.0),
                    band: Some("SurfReflect_M1_1".to_owned()),
                }),
                refs: vec![
                    ChunkRef {
                        key: "0.0".to_owned(),
                        path: "VNP09GA.A2012019.h33v12.002.2023122182434.h5".to_owned(),
                        offset: 40_381,
                        length: 812_345,
                    },
                    ChunkRef {
                        key: "0.1".to_owned(),
                        path: "VNP09GA.A2012019.h33v12.002.2023122182434.h5".to_owned(),
                        offset: 852_726,
                        length: 630_002,
                    },
                ],
            },
            VirtualArray {
                name: "utm_example".to_owned(),
                shape: vec![512, 512],
                chunks: vec![512, 512],
                dtype: "uint16".to_owned(),
                codecs: vec![],
                georef: Some(Georef {
                    crs: GeorefCrs::Epsg(32613),
                    transform: GeoTransform::north_up(453_720.0, 4_353_960.0, 30.0, -30.0),
                    nodata: None,
                    band: None,
                }),
                refs: vec![ChunkRef {
                    key: "0.0".to_owned(),
                    path: "granule.h5".to_owned(),
                    offset: 2_048,
                    length: 524_288,
                }],
            },
            VirtualArray {
                name: "HDFEOS INFORMATION/StructMetadata.0".to_owned(),
                shape: vec![],
                chunks: vec![],
                dtype: "|S32000".to_owned(),
                codecs: vec![],
                georef: None,
                refs: vec![ChunkRef {
                    key: String::new(),
                    path: "granule.h5".to_owned(),
                    offset: 4_096,
                    length: 32_000,
                }],
            },
        ],
    }
}

#[test]
fn representative_manifest_document_is_pinned() {
    let value: serde_json::Value =
        serde_json::from_str(&representative().to_json_string()).unwrap();
    insta::assert_json_snapshot!("virtual_manifest_v1", value);
}

#[test]
fn manifest_round_trips_through_text() {
    let m = representative();
    let text = m.to_json_string();
    assert_eq!(VirtualManifest::from_json_str(&text).unwrap(), m);
}
