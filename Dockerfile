# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# The swath image (issue #29, ARCHITECTURE.md §15): multi-stage — a pinned
# Node stage builds the production web bundle (issue #103), the pinned Rust
# toolchain embeds it into the release binary (feature `embedded-ui`, on by
# default), and a slim Debian runtime carries the binary plus the committed
# HLS fixtures (so `--fixtures` serves the demo layers AND the UI with zero
# external data). Runtime base is debian-slim rather than distroless: the
# compose healthcheck needs curl in the container, and ca-certificates covers
# future HTTPS object-store roots. The toolchain tag tracks rust-toolchain.toml;
# the Node major tracks web/package.json's devEngines.

FROM node:24-trixie-slim AS web
WORKDIR /src/web
# pnpm at the version package.json pins (packageManager); corepack ships
# with Node 24 and reads that field.
RUN corepack enable
# Dependency layer first so source edits don't re-install.
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm run build

FROM rust:1.97.1-slim-trixie AS build
# cmake + make: the production referencer statically bundles libhdf5
# (hdf5-metno-src builds the C library from source — ADR 0006's
# single-binary story; no system HDF5 in the runtime image).
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake make \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# The production bundle where swath-cli's build script stages it from
# (web/dist — .dockerignore keeps the context's own dist out, so the
# embedded UI is always THIS build's).
COPY --from=web /src/web/dist /src/web/dist
# --locked: the committed Cargo.lock is the build, exactly as in CI.
RUN cargo build --release --locked -p swath-cli

FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/swath /usr/local/bin/swath
# The committed demo fixtures, at the path `--fixtures` expects relative to
# the workdir (./tests/fixtures).
COPY tests/fixtures /app/tests/fixtures
# Unprivileged (ports >1024 only).
USER 65534:65534
EXPOSE 8080
# Bind via env, not only the CMD flag: user-supplied `docker run` args
# REPLACE CMD, so the README's `docker run … serve --fixtures` would
# otherwise bind the in-container loopback and be unreachable through -p.
ENV SWATH_BIND=0.0.0.0:8080
ENTRYPOINT ["swath"]
CMD ["serve", "--fixtures", "--bind", "0.0.0.0:8080"]
