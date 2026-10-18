use crate::LogStore;
use crate::store::SharedState;

/// What the HTTP handlers read besides Raft itself.
pub struct AppData {
    /// The state machine's applied state.
    pub state: SharedState,

    /// A handle on the log store, for reporting the retained log ids.
    pub log_store: LogStore,
}

pub type App = app_http::App<crate::TypeConfig, crate::StateMachineStore, AppData>;
