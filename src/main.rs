use std::process::ExitCode;
use std::sync::Arc;

use azure_core::credentials::TokenCredential;
use azure_identity::DeveloperToolsCredential;
use clap::Parser;
use cosmoscli::app::App;
use cosmoscli::arm::Arm;
use cosmoscli::cli::Cli;
use cosmoscli::cosmos::CosmosDataPlane;
use cosmoscli::prompt::TerminalConfirm;

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
    let credential: Arc<dyn TokenCredential> = DeveloperToolsCredential::new(None)?;
    let mut app = App {
        management: Arm::new(credential.clone()),
        data: CosmosDataPlane { entra: credential },
        input: Box::new(std::io::stdin()),
        confirm: Box::new(TerminalConfirm),
        out: std::io::stdout().lock(),
        global: cli.global,
    };
    app.run(cli.command).await
}
