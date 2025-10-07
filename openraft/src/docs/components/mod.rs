//! Components and sub systems of the openraft project.
//!
//! - [`Engine and Runtime`](engine_runtime)
//! - [`StateMachine`](state_machine)

pub mod engine_runtime {
    #![doc = include_str!("engine-runtime.md")]
}

pub mod state_machine {
    #![doc = include_str!("state-machine.md")]
}
