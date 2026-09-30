use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use azure_core::credentials::TokenCredential;
use azure_identity::DeveloperToolsCredential;
use clap::Parser;
use cosmos_core::arm::Arm;
use cosmos_core::cache::AccountCache;
use cosmos_core::connector::Connector;
use cosmos_core::cosmos::{CosmosDataPlane, skip_vm_metadata_probe};
use cosmos_core::credential::{CachedCredential, prefetch_tokens};
use cosmos_core::management::Management;
use cosmoscli::app::App;
use cosmoscli::cli::{Cli, GlobalArgs};
use cosmoscli::credential::scopes_needed;
use cosmoscli::interactive::{AccountList, Repl, SystemClock};
use cosmoscli::prompt::{TerminalConfirm, TerminalLines, TerminalPicker};

fn main() -> ExitCode {
    let cli = Cli::parse();
    // SAFETY: called before the runtime starts, while this is the only thread
    unsafe { skip_vm_metadata_probe() };
    let result = tokio::runtime::Runtime::new()
        .map_err(anyhow::Error::from)
        .and_then(|runtime| runtime.block_on(run(cli)));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let credential: Arc<dyn TokenCredential> =
        Arc::new(CachedCredential::new(DeveloperToolsCredential::new(None)?));
    prefetch_tokens(
        &credential,
        scopes_needed(cli.command.as_ref(), &cli.global),
    );
    match cli.command {
        Some(command) => {
            let mut app = app(credential, cli.global, std::io::stdout().lock());
            app.run(command).await
        }
        None => {
            let subscription = cli.global.subscription.clone();
            let accounts = prefetch_accounts(&credential, subscription.clone());
            // Unlocked stdout, so the line editor and pickers can draw on the terminal too
            let app = app(credential, cli.global, std::io::stdout());
            println!("Type /help for commands, /exit or Ctrl-C to leave.");
            let mut repl = Repl::new(
                app,
                Box::new(TerminalLines::new()?),
                Box::new(TerminalPicker),
                Box::new(SystemClock),
            )
            .with_account_prefetch(accounts);
            if let Some(cache) = AccountCache::for_user(subscription.as_deref()) {
                repl = repl.with_account_cache(cache);
            }
            repl.run().await
        }
    }
}

/// Starts listing accounts in the background, so the first `/accounts` finds them ready.
/// The list is also cached for the next run, even when `/accounts` is never used.
fn prefetch_accounts(
    credential: &Arc<dyn TokenCredential>,
    subscription: Option<String>,
) -> AccountList {
    let arm = Arm::new(credential.clone());
    let listing = tokio::spawn(async move {
        let accounts = arm.list_accounts(subscription.as_deref()).await?;
        if let Some(cache) = AccountCache::for_user(subscription.as_deref()) {
            // A cache that cannot be written only costs the next run its head start
            let _ = cache.save(&accounts);
        }
        Ok(accounts)
    });
    Box::pin(async move { listing.await? })
}

fn app<W: Write>(
    credential: Arc<dyn TokenCredential>,
    global: GlobalArgs,
    out: W,
) -> App<Arm, CosmosDataPlane, W> {
    App {
        connector: Connector::new(
            Arm::new(credential.clone()),
            CosmosDataPlane::new(credential),
            global.settings(),
        ),
        input: Box::new(std::io::stdin()),
        confirm: Box::new(TerminalConfirm),
        out,
    }
}
