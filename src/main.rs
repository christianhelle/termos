mod arm;
mod cache;
mod clipboard;
mod connector;
mod cosmos;
mod credential;
#[allow(dead_code)] // Used by the query editor as it comes together
mod editor;
mod effects;
mod emulator;
mod input;
mod json;
mod management;
mod partition;
mod query;
#[allow(dead_code)] // Used by the query editor as it comes together
mod sql;
mod state;
mod store;
#[cfg(test)]
mod testing;
mod ui;

use std::io::stdout;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use crate::arm::Arm;
use crate::cache::AccountCache;
use crate::connector::{Connector, Settings};
use crate::cosmos::{CosmosDataPlane, skip_vm_metadata_probe};
use crate::credential::{COSMOS_SCOPE, CachedCredential, MANAGEMENT_SCOPE, prefetch_tokens};
use crate::emulator::{Emulator, trust_emulator_certificate};
use crate::management::{Account, Management};
use crate::store::AuthMode;
use azure_core::credentials::TokenCredential;
use azure_identity::DeveloperToolsCredential;
use clap::Parser;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as TermEvent, EventStream, KeyEventKind,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use futures::StreamExt;
use ratatui::DefaultTerminal;
use ratatui::layout::{Position, Size};
use tokio::sync::mpsc;

use crate::effects::Runner;
use crate::state::{AppState, Effect, Event, Mouse, MouseAction, Msg, update};

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

    /// Browse the local Cosmos DB emulator instead of Azure, at https://localhost:8081/ unless given
    #[arg(
        long,
        value_name = "ENDPOINT",
        num_args = 0..=1,
        default_missing_value = emulator::DEFAULT_ENDPOINT,
        conflicts_with_all = ["subscription", "auth"]
    )]
    emulator: Option<String>,
}

type AppRunner<M> = Runner<M, CosmosDataPlane>;

fn main() -> ExitCode {
    let args = Args::parse();
    // SAFETY: called before the runtime starts, while this is the only thread
    unsafe { skip_vm_metadata_probe() };
    if let Some(endpoint) = &args.emulator {
        // SAFETY: called before the runtime starts, while this is the only thread
        unsafe { trust_emulator_certificate(endpoint) };
    }
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
    if let Some(endpoint) = args.emulator {
        return run_emulator(&endpoint, args.key, credential).await;
    }
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
    show(runner, cached).await
}

/// Browses the emulator with its key, without Resource Manager or a cached account list.
async fn run_emulator(
    endpoint: &str,
    key: Option<String>,
    credential: Arc<dyn TokenCredential>,
) -> anyhow::Result<()> {
    let key = key.unwrap_or_else(|| emulator::DEFAULT_KEY.to_string());
    let settings = Settings {
        subscription: None,
        auth: AuthMode::Key,
        key: Some(key.clone()),
    };
    let runner = Runner::new(Connector::new(
        Emulator::new(endpoint, key)?,
        CosmosDataPlane::new(credential),
        settings,
    ));
    show(runner, None).await
}

/// Runs the terminal UI until the user quits, then restores the terminal.
async fn show<M: Management + 'static>(
    runner: AppRunner<M>,
    cached: Option<Vec<Account>>,
) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = match execute!(stdout(), EnableMouseCapture) {
        Ok(()) => event_loop(&mut terminal, Rc::new(runner), cached).await,
        Err(error) => Err(error.into()),
    };
    // Restore the terminal even when the mouse could not be released
    let released = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result.and(released.map_err(Into::into))
}

/// Draws the state, then waits for a key, the mouse or finished work, until the user quits.
async fn event_loop<M: Management + 'static>(
    terminal: &mut DefaultTerminal,
    runner: Rc<AppRunner<M>>,
    cached_accounts: Option<Vec<Account>>,
) -> anyhow::Result<()> {
    let (sender, mut finished) = mpsc::unbounded_channel();
    let mut keys = EventStream::new();
    let mut left_held = false;
    let (mut state, effects) = AppState::with_cached_accounts(cached_accounts);
    start(&runner, &sender, effects);
    while !state.quit {
        state.doc_height = ui::document_height(terminal.size()?, state.zoomed_pane());
        state.results_height = ui::results_height(terminal.size()?, state.zoomed_pane());
        state.tree_height = ui::tree_height(terminal.size()?);
        terminal.draw(|frame| ui::draw(frame, &state))?;
        let event = tokio::select! {
            Some(input) = keys.next() => match input? {
                // Windows also reports key releases
                TermEvent::Key(key) if key.kind != KeyEventKind::Release => Event::Key(key),
                TermEvent::Mouse(mouse) => match pane_mouse(terminal.size()?, mouse, &state, &mut left_held) {
                    Some(mouse) => Event::Mouse(mouse),
                    None => continue,
                },
                _ => continue,
            },
            Some(msg) = finished.recv() => Event::Msg(msg),
        };
        let effects = update(&mut state, event);
        start(&runner, &sender, effects);
    }
    Ok(())
}

/// A left click, drag or release or turn of the wheel, in terms of the pane it is over.
///
/// `left_held` remembers whether the left button is down between events.
fn pane_mouse(
    size: Size,
    mouse: MouseEvent,
    state: &AppState,
    left_held: &mut bool,
) -> Option<Mouse> {
    let action = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            *left_held = true;
            MouseAction::Click
        }
        MouseEventKind::Up(MouseButton::Left) => {
            *left_held = false;
            MouseAction::Release
        }
        MouseEventKind::Drag(MouseButton::Left) => MouseAction::Drag,
        // Windows Terminal reports a move with the button held as a plain move
        MouseEventKind::Moved if *left_held => MouseAction::Drag,
        MouseEventKind::ScrollUp => MouseAction::ScrollUp,
        MouseEventKind::ScrollDown => MouseAction::ScrollDown,
        _ => return None,
    };
    let (pane, at) = ui::pane_at(
        size,
        Position::new(mouse.column, mouse.row),
        state.tree_hidden,
        state.zoomed_pane(),
    )?;
    Some(Mouse { action, pane, at })
}

/// Starts background work, which reports back through the sender when done.
fn start<M: Management + 'static>(
    runner: &Rc<AppRunner<M>>,
    sender: &mpsc::UnboundedSender<Msg>,
    effects: Vec<Effect>,
) {
    for effect in effects {
        let runner = runner.clone();
        let sender = sender.clone();
        tokio::task::spawn_local(async move {
            // The receiver only goes away when the app quits
            let _ = sender.send(runner.run(effect).await);
        });
    }
}
