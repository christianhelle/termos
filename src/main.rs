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
use cosmoscli::credential::CachedCredential;
use cosmoscli::interactive::{Repl, SystemClock};
use cosmoscli::prompt::{TerminalConfirm, TerminalLines, TerminalPicker};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
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
    }
}
