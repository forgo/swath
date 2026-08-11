# Legacy virtual-reference mechanism

Hand-crafted SVG (plan decision #3 — this figure set the style the other three diagrams now
follow; here because the chunk-grid/byte-range geometry is the point). The file
[`virtual-reference.svg`](virtual-reference.svg) is both the editable source and the export.
Every figure on it is traceable via
[`virtual-reference.notes.md`](virtual-reference.notes.md).

![Legacy virtual reference: the original VNP09GA HDF5 granule is never rewritten. At ingest
time SwathReferencer walks the chunk index (no pixel data touched, 14 ms warm / 29 ms cold)
and emits VirtualManifest v1: 67 arrays, 1,551 chunk refs of key/path/offset/length, codec
chain and sinusoidal georef. The manifest is cross-checked ref-for-ref against the
VirtualiZarr sidecar reference: 67/67 arrays, 1,551/1,551 refs, per-chunk offset+length
identical. At serve time VirtualSource intersects the tile window with the chunk grid and
get_range-fetches only the touched chunks from the original .h5, decodes codecs in reverse,
then warps, composes, and encodes — the Trace's provenance is exactly those byte ranges, and
the pixels are proven byte-identical to h5py reads by SHA-256 in
CI.](virtual-reference.svg)

The one-sentence thesis, from the adapter's own rustdoc: legacy granules are served as byte
ranges into the original, untouched file — never the whole granule, never a rewritten copy.
