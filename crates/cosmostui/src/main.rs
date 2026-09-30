mod effects;
mod input;
mod json;
mod query;
mod state;
mod ui;

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use azure_core::credentials::TokenCredential;
use azure_identity::DeveloperToolsCredential;
use clap::Parser;
use cosmos_core::arm::Arm;
use cosmos_core::connector::{Connector, Settings};
use cosmos_core::cosmos::{CosmosDataPlane, skip_vm_metadata_probe};
use cosmos_core::credential::{COSMOS_SCOPE, CachedCredential, MANAGEMENT_SCOPE, prefetch_tokens};
use cosmos_core::store::AuthMode;
use crossterm::event::{Event as TermEvent, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::state::{AppState, Effect, Event, Msg, update};

#[derive(Parser, Debug)]
#[command(
    name = "cosmostui",
    version,
    about = "Terminal user interface for Azure Cosmos DB"
)]
struct Args {
    /// Only use this subscription id instead of every subscription you can access
    #[arg(long)]
    subscription: Option<String>,

    /// How to authenticate to the Cosmos DB data plane
    #[arg(long, value_enum, default_value_t = AuthMode::Auto)]
    auth: AuthMode,

    /// Account key to use instead of fetching one from Resource Manager
    #[arg(long)]
    key: Option<String>,
}

type AppConnector = Connector<Arm, CosmosDataPlane>;

fn main() -> ExitCode {
    let args = Args::parse();
    // SAFETY: called before the runtime starts, while this is the only thread
    unsafe { skip_vm_metadata_probe() };
    let result = tokio::runtime::Runtime::new()
        .map_err(anyhow::Error::from)
        .and_then(|runtime| {
            // Connections are shared with Rc, so background work stays on this thread
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, run(args))
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> anyhow::Result<()> {
    let credential: Arc<dyn TokenCredential> =
        Arc::new(CachedCredential::new(DeveloperToolsCredential::new(None)?));
    let settings = Settings {
        subscription: args.subscription,
        auth: args.auth,
        key: args.key,
    };
    let documents_use_entra = settings.key.is_none() && settings.auth != AuthMode::Key;
    if documents_use_entra {
        prefetch_tokens(&credential, vec![MANAGEMENT_SCOPE, COSMOS_SCOPE]);
    } else {
        prefetch_tokens(&credential, vec![MANAGEMENT_SCOPE]);
    }
    let connector = Rc::new(Connector::new(
        Arm::new(credential.clone()),
        CosmosDataPlane::new(credential),
        settings,
    ));
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, connector).await;
    ratatui::restore();
    result
}

/// Draws the state, then waits for a key or finished work, until the user quits.
async fn event_loop(
    terminal: &mut DefaultTerminal,
    connector: Rc<AppConnector>,
) -> anyhow::Result<()> {
    let (sender, mut finished) = mpsc::unbounded_channel();
    let mut keys = EventStream::new();
    let (mut state, effects) = AppState::new();
    start(&connector, &sender, effects);
    while !state.quit {
        terminal.draw(|frame| ui::draw(frame, &state))?;
        let event = tokio::select! {
            Some(input) = keys.next() => match input? {
                // Windows also reports key releases
                TermEvent::Key(key) if key.kind != KeyEventKind::Release => Event::Key(key),
                _ => continue,
            },
            Some(msg) = finished.recv() => Event::Msg(msg),
        };
        let effects = update(&mut state, event);
        start(&connector, &sender, effects);
    }
    Ok(())
}

/// Starts background work, which reports back through the sender when done.
fn start(connector: &Rc<AppConnector>, sender: &mpsc::UnboundedSender<Msg>, effects: Vec<Effect>) {
    for effect in effects {
        let connector = connector.clone();
        let sender = sender.clone();
        tokio::task::spawn_local(async move {
            // The receiver only goes away when the app quits
            let _ = sender.send(effects::run(&connector, effect).await);
        });
    }
}
