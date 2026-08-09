# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

"""Chunk-grid math shared by manifest generation and the equivalence harness.

A virtual manifest addresses chunks by dotted index keys (``"0.0"``, ``"1.2"`` —
see prototype 0001 §3). The key set is fully determined by array shape and
chunk shape; getting it wrong silently drops or invents byte-ranges, so this
tiny module is property-tested hard.
"""

from collections.abc import Iterator
from itertools import product


def chunk_grid(shape: tuple[int, ...], chunks: tuple[int, ...]) -> tuple[int, ...]:
    """Number of chunks along each dimension (ceiling division).

    Raises ``ValueError`` for mismatched ranks, non-positive chunk sizes, or
    negative dimension sizes. A zero-length dimension yields zero chunks along
    that axis (and therefore an empty grid), matching Zarr semantics.
    """
    if len(shape) != len(chunks):
        msg = f"rank mismatch: shape {shape} vs chunks {chunks}"
        raise ValueError(msg)
    if any(c <= 0 for c in chunks):
        msg = f"chunk sizes must be positive: {chunks}"
        raise ValueError(msg)
    if any(s < 0 for s in shape):
        msg = f"dimension sizes must be non-negative: {shape}"
        raise ValueError(msg)
    return tuple(-(-s // c) for s, c in zip(shape, chunks, strict=True))


def chunk_keys(shape: tuple[int, ...], chunks: tuple[int, ...]) -> Iterator[str]:
    """Dotted manifest keys for every chunk, row-major (last dimension fastest)."""
    grid = chunk_grid(shape, chunks)
    for idx in product(*(range(n) for n in grid)):
        yield ".".join(str(i) for i in idx)
