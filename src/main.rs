use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use azure_core::credentials::TokenCredential;
use azure_identity::DeveloperToolsCredential;
use clap::Parser;
use cosmoscli::app::App;
use cosmoscli::arm::Arm;
use cosmoscli::cli::{Cli, GlobalArgs};
use cosmoscli::cosmos::CosmosDataPlane;
use cosmoscli::credential::{CachedCredential, scopes_needed};
use cosmoscli::interactive::{Repl, SystemClock};
use cosmoscli::prompt::{TerminalConfirm, TerminalLines, TerminalPicker};

fn main() -> ExitCode {
    let cli = Cli::parse();
    skip_vm_metadata_probe();
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

/// Stops the Cosmos DB SDK probing the Azure VM metadata service, which it only uses
/// for diagnostics. Off Azure the probe waits 2 seconds to time out on the first connection.
fn skip_vm_metadata_probe() {
    if std::env::var_os("COSMOS_DISABLE_IMDS").is_none() {
        // SAFETY: called before the runtime starts, while this is the only thread
        unsafe { std::env::set_var("COSMOS_DISABLE_IMDS", "1") };
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
            // Unlocked stdout, so the line editor and pickers can draw on the terminal too
            let app = app(credential, cli.global, std::io::stdout());
            println!("Type /help for commands, /exit or Ctrl-C to leave.");
            let mut repl = Repl::new(
                app,
                Box::new(TerminalLines::new()?),
                Box::new(TerminalPicker),
                Box::new(SystemClock),
            );
            repl.run().await
        }
    }
}

/// Fetches tokens in the background, so the first requests find them in the cache.
fn prefetch_tokens(credential: &Arc<dyn TokenCredential>, scopes: Vec<&'static str>) {
    for scope in scopes {
        let credential = credential.clone();
        tokio::spawn(async move {
            // A failure here resurfaces with context when the token is really needed
            let _ = credential.get_token(&[scope], None).await;
        });
    }
}

fn app<W: Write>(
    credential: Arc<dyn TokenCredential>,
    global: GlobalArgs,
    out: W,
) -> App<Arm, CosmosDataPlane, W> {
    App {
        management: Arm::new(credential.clone()),
        data: CosmosDataPlane { entra: credential },
        input: Box::new(std::io::stdin()),
        confirm: Box::new(TerminalConfirm),
        out,
        global,
        known_accounts: Default::default(),
        connections: Default::default(),
        account_keys: Default::default(),
    }
}
