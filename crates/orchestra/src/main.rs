//! `orchestra`: one binary for the daemon, the dashboard and the CLI.

mod cli;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

fn main() -> Result<()> {
    // Rust ignores SIGPIPE, so `orchestra usage | head` panics on the closed
    // pipe instead of ending quietly. Restore the usual behaviour.
    #[cfg(unix)]
    unsafe {
        libc_signal_default();
    }

    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(cli.run())
}

/// Restore the default disposition of `SIGPIPE`.
#[cfg(unix)]
unsafe fn libc_signal_default() {
    // SIG_DFL on SIGPIPE: the process ends when its reader goes away.
    nix::sys::signal::signal(
        nix::sys::signal::Signal::SIGPIPE,
        nix::sys::signal::SigHandler::SigDfl,
    )
    .ok();
}
