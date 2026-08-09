//! The virtual-reference manifest model, its (de)serialization, and the equivalence check.
//!
//! The prototype JSON stands in for the production form (Icechunk virtual chunk references). The
//! equivalence check is the reusable artifact: it is the conformance test that lets us swap reference
//! generators (Python vs Rust) safely — equivalent manifests ⇒ interchangeable behind the port (ADR 0006).

use crate::json::{Json, parse};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct VirtualManifest {
    pub generator: String,
    pub source: String,
    pub arrays: Vec<ArrayRef>,
}

#[derive(Debug, Clone)]
pub struct ArrayRef {
    pub name: String,
    pub shape: Vec<u64>,
    pub chunks: Vec<u64>,
    pub dtype: String,
    pub codecs: Vec<String>,
    pub refs: Vec<ChunkRef>,
}

#[derive(Debug, Clone)]
pub struct ChunkRef {
    pub key: String,
    pub path: String,
    pub offset: u64,
    pub length: u64,
}

// ---------- deserialization ----------

impl VirtualManifest {
    pub fn from_str(s: &str) -> Result<VirtualManifest, String> {
        Self::from_json(&parse(s)?)
    }

    pub fn from_json(v: &Json) -> Result<VirtualManifest, String> {
        let generator = v
            .get("generator")
            .and_then(Json::as_str)
            .unwrap_or("unknown")
            .to_string();
        let source = v
            .get("source")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        let arrays_j = v
            .get("arrays")
            .and_then(Json::as_array)
            .ok_or("manifest missing 'arrays' array")?;
        let mut arrays = Vec::with_capacity(arrays_j.len());
        for a in arrays_j {
            arrays.push(ArrayRef::from_json(a)?);
        }
        Ok(VirtualManifest {
            generator,
            source,
            arrays,
        })
    }
}

impl ArrayRef {
    fn from_json(v: &Json) -> Result<ArrayRef, String> {
        let name = v
            .get("name")
            .and_then(Json::as_str)
            .ok_or("array missing 'name'")?
            .to_string();
        let dtype = v
            .get("dtype")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        let shape = u64_vec(v.get("shape"));
        let chunks = u64_vec(v.get("chunks"));
        let codecs = str_vec(v.get("codecs"));
        let refs_j = v.get("refs").and_then(Json::as_array).unwrap_or(&[]);
        let mut refs = Vec::with_capacity(refs_j.len());
        for r in refs_j {
            refs.push(ChunkRef {
                key: r
                    .get("key")
                    .and_then(Json::as_str)
                    .ok_or("ref missing 'key'")?
                    .to_string(),
                path: r
                    .get("path")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                offset: r
                    .get("offset")
                    .and_then(Json::as_u64)
                    .ok_or("ref missing 'offset'")?,
                length: r
                    .get("length")
                    .and_then(Json::as_u64)
                    .ok_or("ref missing 'length'")?,
            });
        }
        Ok(ArrayRef {
            name,
            shape,
            chunks,
            dtype,
            codecs,
            refs,
        })
    }
}

fn u64_vec(v: Option<&Json>) -> Vec<u64> {
    v.and_then(Json::as_array)
        .map(|a| a.iter().filter_map(Json::as_u64).collect())
        .unwrap_or_default()
}
fn str_vec(v: Option<&Json>) -> Vec<String> {
    v.and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// ---------- serialization (direct string building) ----------

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}
fn u64_arr(v: &[u64]) -> String {
    let parts: Vec<String> = v.iter().map(|n| n.to_string()).collect();
    format!("[{}]", parts.join(", "))
}
fn str_arr(v: &[String]) -> String {
    let parts: Vec<String> = v.iter().map(|s| format!("\"{}\"", esc(s))).collect();
    format!("[{}]", parts.join(", "))
}

impl VirtualManifest {
    pub fn to_json_string(&self) -> String {
        let mut s = String::new();
        s.push_str("{\n");
        s.push_str(&format!("  \"generator\": \"{}\",\n", esc(&self.generator)));
        s.push_str(&format!("  \"source\": \"{}\",\n", esc(&self.source)));
        s.push_str("  \"arrays\": [\n");
        for (i, a) in self.arrays.iter().enumerate() {
            s.push_str("    {\n");
            s.push_str(&format!("      \"name\": \"{}\",\n", esc(&a.name)));
            s.push_str(&format!("      \"shape\": {},\n", u64_arr(&a.shape)));
            s.push_str(&format!("      \"chunks\": {},\n", u64_arr(&a.chunks)));
            s.push_str(&format!("      \"dtype\": \"{}\",\n", esc(&a.dtype)));
            s.push_str(&format!("      \"codecs\": {},\n", str_arr(&a.codecs)));
            s.push_str("      \"refs\": [\n");
            for (j, r) in a.refs.iter().enumerate() {
                let comma = if j + 1 < a.refs.len() { "," } else { "" };
                s.push_str(&format!(
                    "        {{ \"key\": \"{}\", \"path\": \"{}\", \"offset\": {}, \"length\": {} }}{}\n",
                    esc(&r.key), esc(&r.path), r.offset, r.length, comma
                ));
            }
            s.push_str("      ]\n");
            let comma = if i + 1 < self.arrays.len() { "," } else { "" };
            s.push_str(&format!("    }}{}\n", comma));
        }
        s.push_str("  ]\n}\n");
        s
    }
}

// ---------- equivalence ----------

#[derive(Debug, Default)]
pub struct EquivalenceReport {
    pub arrays_a: usize,
    pub arrays_b: usize,
    pub matched_arrays: usize,
    pub grid_mismatches: Vec<String>,
    pub chunk_mismatches: Vec<String>,
}

impl EquivalenceReport {
    pub fn equivalent(&self) -> bool {
        self.arrays_a == self.arrays_b
            && self.matched_arrays == self.arrays_a
            && self.grid_mismatches.is_empty()
            && self.chunk_mismatches.is_empty()
    }
}

/// Compare two manifests: same arrays, same chunk grid + dtype, same per-chunk (offset, length).
pub fn compare(a: &VirtualManifest, b: &VirtualManifest) -> EquivalenceReport {
    let mut r = EquivalenceReport {
        arrays_a: a.arrays.len(),
        arrays_b: b.arrays.len(),
        ..Default::default()
    };
    for arr_a in &a.arrays {
        let Some(arr_b) = b.arrays.iter().find(|x| x.name == arr_a.name) else {
            r.grid_mismatches
                .push(format!("array '{}' missing in B", arr_a.name));
            continue;
        };
        r.matched_arrays += 1;
        if arr_a.shape != arr_b.shape || arr_a.chunks != arr_b.chunks || arr_a.dtype != arr_b.dtype
        {
            r.grid_mismatches.push(format!(
                "array '{}' grid/dtype differs: A(shape={:?},chunks={:?},dtype={}) vs B(shape={:?},chunks={:?},dtype={})",
                arr_a.name, arr_a.shape, arr_a.chunks, arr_a.dtype, arr_b.shape, arr_b.chunks, arr_b.dtype
            ));
        }
        let map_b: BTreeMap<&str, &ChunkRef> =
            arr_b.refs.iter().map(|c| (c.key.as_str(), c)).collect();
        for c_a in &arr_a.refs {
            match map_b.get(c_a.key.as_str()) {
                None => r
                    .chunk_mismatches
                    .push(format!("{}::{} missing in B", arr_a.name, c_a.key)),
                Some(c_b) => {
                    if c_a.offset != c_b.offset || c_a.length != c_b.length {
                        r.chunk_mismatches.push(format!(
                            "{}::{} offset/length differs: A({},{}) vs B({},{})",
                            arr_a.name, c_a.key, c_a.offset, c_a.length, c_b.offset, c_b.length
                        ));
                    }
                }
            }
        }
    }
    r
}
