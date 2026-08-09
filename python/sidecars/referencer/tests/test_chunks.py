# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

"""Property tests for chunk-grid math (mirrors the Rust side of the port, prototype 0001)."""

import math

import pytest
from hypothesis import given
from hypothesis import strategies as st
from swath_referencer import chunk_grid, chunk_keys

# Realistic ranks (1-4) and sizes; small enough to enumerate keys exhaustively.
dims = st.integers(min_value=0, max_value=64)
chunk_sizes = st.integers(min_value=1, max_value=64)


@st.composite
def shapes_and_chunks(draw: st.DrawFn) -> tuple[tuple[int, ...], tuple[int, ...]]:
    rank = draw(st.integers(min_value=1, max_value=4))
    shape = tuple(draw(dims) for _ in range(rank))
    chunks = tuple(draw(chunk_sizes) for _ in range(rank))
    return shape, chunks


@given(shapes_and_chunks())
def test_key_count_equals_grid_product(sc: tuple[tuple[int, ...], tuple[int, ...]]) -> None:
    shape, chunks = sc
    grid = chunk_grid(shape, chunks)
    keys = list(chunk_keys(shape, chunks))
    assert len(keys) == math.prod(grid)
    assert len(set(keys)) == len(keys), "keys must be unique"


@given(shapes_and_chunks())
def test_every_element_is_covered(sc: tuple[tuple[int, ...], tuple[int, ...]]) -> None:
    shape, chunks = sc
    grid = chunk_grid(shape, chunks)
    # ceiling division: enough chunks to cover every index, none entirely past the end
    for size, chunk, n in zip(shape, chunks, grid, strict=True):
        assert n * chunk >= size
        if size > 0:
            assert (n - 1) * chunk < size


@given(shapes_and_chunks())
def test_keys_parse_back_within_grid(sc: tuple[tuple[int, ...], tuple[int, ...]]) -> None:
    shape, chunks = sc
    grid = chunk_grid(shape, chunks)
    for key in chunk_keys(shape, chunks):
        indices = tuple(int(part) for part in key.split("."))
        assert len(indices) == len(grid)
        assert all(0 <= i < n for i, n in zip(indices, grid, strict=True))


def test_single_chunk_when_chunks_exceed_shape() -> None:
    assert chunk_grid((10, 10), (100, 100)) == (1, 1)
    assert list(chunk_keys((10, 10), (100, 100))) == ["0.0"]


def test_known_viirs_like_grid() -> None:
    # The prototype's example: 3232x3200 array in 1616x1600 chunks -> 2x2 grid.
    assert chunk_grid((3232, 3200), (1616, 1600)) == (2, 2)
    assert list(chunk_keys((3232, 3200), (1616, 1600))) == ["0.0", "0.1", "1.0", "1.1"]


@pytest.mark.parametrize(
    ("shape", "chunks"),
    [((10,), (10, 10)), ((10, 10), (0, 10)), ((10, 10), (-1, 10)), ((-1,), (1,))],
)
def test_invalid_inputs_raise(shape: tuple[int, ...], chunks: tuple[int, ...]) -> None:
    with pytest.raises(ValueError, match=r"mismatch|positive|non-negative"):
        chunk_grid(shape, chunks)
