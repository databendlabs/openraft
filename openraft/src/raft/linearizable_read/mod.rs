//! Defines the linearizable read protocol.

mod linearize_state;
mod linearizer;

pub use linearize_state::LinearizeState;
pub use linearizer::Linearizer;
