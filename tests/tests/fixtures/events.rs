use std::sync::Arc;
use std::sync::Mutex;

use openraft::async_runtime::WatchReceiver;
use openraft::async_runtime::WatchSender;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::WatchSenderOf;
use openraft_memstore::TypeConfig;

use crate::fixtures::WAIT_TIMEOUT;

/// Records complete callback events and waits for them without polling sleeps.
#[derive(Clone)]
pub struct Events<T> {
    values: Arc<Mutex<Vec<T>>>,
    count: WatchSenderOf<TypeConfig, usize>,
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        let (count, _) = TypeConfig::watch_channel(0);
        Self {
            values: Default::default(),
            count,
        }
    }
}

impl<T> Events<T>
where T: Clone
{
    /// Record an event and wake observers of the new event count.
    pub fn push(&self, value: T) {
        let count = {
            let mut values = self.values.lock().unwrap();
            values.push(value);
            values.len()
        };
        self.count.send_if_greater(count);
    }

    /// Read the complete recorded event sequence.
    pub fn get(&self) -> Vec<T> {
        let values = self.values.lock().unwrap();
        values.clone()
    }

    /// Wait for at least `count` events within the shared deadline.
    pub async fn wait(&self, count: usize) -> anyhow::Result<Vec<T>> {
        let mut receiver = self.count.subscribe();
        let received = receiver.wait_until_ge(&count);
        TypeConfig::timeout(WAIT_TIMEOUT, received).await??;
        let values = self.get();
        Ok(values)
    }
}
