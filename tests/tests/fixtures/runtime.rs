#![allow(unused_imports)]

#[cfg(feature = "monoio")] pub use local_sync::oneshot;
#[cfg(not(feature = "monoio"))] pub use tokio::sync::oneshot;
