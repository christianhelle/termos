mod arm;
mod cache;
mod connector;
mod cosmos;
mod credential;
mod effects;
mod input;
mod json;
mod management;
mod partition;
mod query;
mod state;
mod store;
#[cfg(test)]
mod testing;
mod ui;

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use crate::arm::Arm;
use crate::cache::AccountCache;
use crate::connector::{Connector, Settings};
use crate::cosmos::{CosmosDataPlane, skip_vm_metadata_probe};
use crate::credential::{COSMOS_SCOPE, CachedCredential, MANAGEMENT_SCOPE, prefetch_tokens};
use crate::management::Account;
use crate::store::AuthMode;
use azure_core::credentials::TokenCredential;
use azure_identity::DeveloperToolsCredential;
use clap::Parser;
use crossterm::event::{Event as TermEvent, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::effects::Runner;
use crate::state::{AppState, Effect, Event, Msg, update};

#[derive(Parser, Debug)]
#[command(
    name = "termos",
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

type AppRunner = Runner<Arm, CosmosDataPlane>;

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
    let cache = AccountCache::for_user(settings.subscription.as_deref());
    let cached = cache.as_ref().and_then(AccountCache::load);
    let mut runner = Runner::new(Connector::new(
        Arm::new(credential.clone()),
        CosmosDataPlane::new(credential),
        settings,
    ));
    if let Some(cache) = cache {
        runner = runner.with_account_cache(cache);
    }
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, Rc::new(runner), cached).await;
    ratatui::restore();
    result
}

/// Draws the state, then waits for a key or finished work, until the user quits.
async fn event_loop(
    terminal: &mut DefaultTerminal,
    runner: Rc<AppRunner>,
    cached_accounts: Option<Vec<Account>>,
) -> anyhow::Result<()> {
    let (sender, mut finished) = mpsc::unbounded_channel();
    let mut keys = EventStream::new();
    let (mut state, effects) = AppState::with_cached_accounts(cached_accounts);
    start(&runner, &sender, effects);
    while !state.quit {
        state.doc_height = ui::document_height(terminal.size()?);
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
        start(&runner, &sender, effects);
    }
    Ok(())
}

/// Starts background work, which reports back through the sender when done.
fn start(runner: &Rc<AppRunner>, sender: &mpsc::UnboundedSender<Msg>, effects: Vec<Effect>) {
    for effect in effects {
        let runner = runner.clone();
        let sender = sender.clone();
        tokio::task::spawn_local(async move {
            // The receiver only goes away when the app quits
            let _ = sender.send(runner.run(effect).await);
        });
    }
}
