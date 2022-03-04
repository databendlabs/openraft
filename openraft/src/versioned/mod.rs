#[allow(clippy::module_inception)]
mod versioned;

pub use versioned::Updatable;
pub use versioned::Update;
pub use versioned::UpdateError;
pub use versioned::Versioned;
