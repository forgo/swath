# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# The swath image (issue #29, ARCHITECTURE.md §15): multi-stage — pinned Rust
# toolchain builds the release binary, a slim Debian runtime carries the binary
# plus the committed HLS fixtures (so `--fixtures` serves the demo layers with
# zero external data). Runtime base is debian-slim rather than distroless: the
# compose healthcheck needs curl in the container, and ca-certificates covers
# future HTTPS object-store roots. The toolchain tag tracks rust-toolchain.toml.

FROM rust:1.97.1-slim-trixie AS build
WORKDIR /src
COPY . .
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
ENTRYPOINT ["swath"]
CMD ["serve", "--fixtures", "--bind", "0.0.0.0:8080"]
