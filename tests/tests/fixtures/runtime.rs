#![allow(unused_imports)]

#[cfg(feature = "monoio")] pub use local_sync::oneshot;
#[cfg(feature = "monoio")] pub use monoio::spawn;
#[cfg(feature = "monoio")] pub use monoio::time::sleep;
#[cfg(feature = "monoio")] pub use openraft::monoio::MonoioInstant as Instant;
#[cfg(feature = "monoio")] pub use openraft::monoio::MonoioRuntime as Runtime;
#[cfg(not(feature = "monoio"))] pub use openraft::TokioInstant as Instant;
#[cfg(not(feature = "monoio"))] pub use openraft::TokioRuntime as Runtime;
#[cfg(not(feature = "monoio"))] pub use tokio::spawn;
#[cfg(not(feature = "monoio"))] pub use tokio::sync::oneshot;
#[cfg(not(feature = "monoio"))] pub use tokio::time::sleep;
