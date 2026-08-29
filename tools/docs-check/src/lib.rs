// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The documentation gates (`just docs-check`): a test-only crate that holds
//! `docs/` to the code — CONFIG.md to the clap tree and serde schema,
//! ENDPOINTS.md to the axum routers, source-fingerprint stamps, deferral
//! pointers, cross-doc claims, measured-number markers, word budgets, and
//! the mutation tests that prove each gate can fail. Moved out of
//! `swath-cli` (#353) so the product crate carries none of it and the gate
//! reaches the binary's `Cli`/`ConfigFile` through its library.

#[cfg(test)]
mod check;
