use std::fmt;

use openraft::async_runtime::WatchReceiver;
use openraft::async_runtime::WatchSender;
use openraft::macros::since;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::WatchSenderOf;

use crate::TypeConfig;

/// Signals when an operation enters and holds it until explicitly released.
#[since(version = "0.10.0")]
#[derive(Clone)]
pub struct OperationGate {
    entered: WatchSenderOf<TypeConfig, bool>,
    released: WatchSenderOf<TypeConfig, bool>,
}

impl Default for OperationGate {
    fn default() -> Self {
        let (entered, _) = TypeConfig::watch_channel(false);
        let (released, _) = TypeConfig::watch_channel(false);
        Self { entered, released }
    }
}

impl fmt::Debug for OperationGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationGate").finish_non_exhaustive()
    }
}

impl OperationGate {
    /// Wait until an operation has entered this gate.
    pub async fn entered(&self) {
        let mut receiver = self.entered.subscribe();
        let received = receiver.wait_until_ge(&true).await;
        received.expect("gate owns the sender");
    }

    /// Release current and future operations waiting at this gate.
    pub fn release(&self) {
        self.released.send_if_different(true);
    }

    /// Signal entry and block until the test releases this gate.
    pub async fn block(&self) {
        self.entered.send_if_different(true);
        let mut receiver = self.released.subscribe();
        let received = receiver.wait_until_ge(&true).await;
        received.expect("gate owns the sender");
    }
}
