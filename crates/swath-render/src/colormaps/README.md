# Vendored colormap LUTs

`luts.json` carries the 256-entry RGB byte lookup tables for the palette
variants of `swath_render::ir::Colormap` (`viridis`, `magma`, `RdYlGn`),
vendored verbatim from **matplotlib 3.10.3** — the pinned reference the
colormap golden-pixel tests (`crates/swath-render/tests/colormaps.rs`)
assert exact RGBA values against.

## Provenance

- Source: matplotlib 3.10.3 (<https://matplotlib.org>), colormap registry
  entries `viridis`, `magma` (256-entry `ListedColormap`s from
  `matplotlib/_cm_listed.py`) and `RdYlGn` (the ColorBrewer diverging map,
  built as a 256-entry `LinearSegmentedColormap` from
  `matplotlib/_cm.py`).
- Extraction: each map sampled at the 256 LUT positions and quantized to
  bytes by matplotlib itself (`bytes=True`), i.e. exactly the byte LUT
  matplotlib renders with. Alpha is 255 everywhere and is not stored.

Regeneration (must reproduce the committed file byte-for-byte):

```sh
uv run --with "matplotlib==3.10.3" python - <<'EOF'
import json, numpy as np, matplotlib as mpl
maps = {}
for name in ["viridis", "magma", "RdYlGn"]:
    rgba = mpl.colormaps[name](np.linspace(0.0, 1.0, 256), bytes=True)
    maps[name] = [[int(r), int(g), int(b)] for r, g, b, _ in rgba]
doc = {"matplotlib_version": mpl.__version__, "entries": 256, "maps": maps}
with open("crates/swath-render/src/colormaps/luts.json", "w") as f:
    json.dump(doc, f, separators=(",", ":"))
    f.write("\n")
EOF
```

## Data licenses

- `viridis` and `magma` were created for matplotlib by Nathaniel J. Smith
  and Stéfan van der Walt and released under **CC0-1.0**
  (<https://github.com/BIDS/colormap>).
- `RdYlGn` is a ColorBrewer palette, Copyright 2002 Cynthia Brewer, Mark
  Harrower, and The Pennsylvania State University, licensed **Apache-2.0**
  (<https://colorbrewer2.org>).

`luts.json` is REUSE-annotated accordingly in the repository's
`REUSE.toml` (`Apache-2.0 AND CC0-1.0`).
