/// Reads go through `Raft::with_state_machine`, so the application holds no data of its own.
pub type App = app_http::App<crate::TypeConfig, crate::StateMachineStore, ()>;
