mod cli;
mod commands;
mod config;

use std::process::ExitCode;

use clap::Parser;
use dv_core::VaultError;

use crate::cli::{Cli, Command};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Domain errors print only the summary, so users never see anyhow's call chain.
            match err.downcast_ref::<VaultError>() {
                Some(vault_err) => eprintln!("error: {vault_err}"),
                None => eprintln!("error: {err:#}"),
            }
            ExitCode::from(exit_code(&err))
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let ctx = config::Context::load(&cli)?;
    match &cli.command {
        Command::Init(args) => commands::init::run(&ctx, args),
        Command::Add(args) => commands::add::run(&ctx, args).await,
        Command::Sync(args) => commands::sync::run(&ctx, args).await,
        Command::List => commands::list::run(&ctx).await,
        Command::Show(args) => commands::show::run(&ctx, args).await,
        Command::Export(args) => commands::export::run(&ctx, args).await,
        Command::Verify(args) => commands::verify::run(&ctx, args).await,
        Command::Gc(args) => commands::gc::run(&ctx, args).await,
        Command::Reindex => commands::reindex::run(&ctx).await,
        Command::Doctor => commands::doctor::run(&ctx),
    }
}

/// Exit code conventions are documented in `docs/design.md` §9.3.
fn exit_code(err: &anyhow::Error) -> u8 {
    match err.downcast_ref::<VaultError>() {
        Some(VaultError::InvalidId(_)) => 2,
        Some(VaultError::Auth) => 3,
        Some(VaultError::Network(_)) => 4,
        Some(VaultError::Corrupt(_)) => 5,
        Some(_) => 1,
        None => 1,
    }
}

fn init_tracing(verbose: u8) {
    let default = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
