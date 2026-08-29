// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `swath` binary: parse, dispatch, exit — everything else is the
//! library (`swath_cli`).

use std::process::ExitCode;

use clap::Parser as _;
use swath_cli::{Cli, Command, init_tracing, report};

fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => report(swath_cli::serve::run(&args)),
        Command::Ingest(args) => report(swath_cli::ingest::run(&args)),
        Command::Materialize(args) => report(swath_cli::materialize::run(&args)),
    }
}
