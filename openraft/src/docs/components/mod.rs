//! Components and sub systems of the openraft project.
//!
//! - [`Engine and Runtime`](engine_runtime)
//! - [`StateMachine`](state_machine)
//! - [`Exposing Raft to clients`](application_api)

pub mod engine_runtime {
    #![doc = include_str!("engine-runtime.md")]
}

pub mod state_machine {
    #![doc = include_str!("state-machine.md")]
}

pub mod application_api {
    #![doc = include_str!("application-api.md")]
}
