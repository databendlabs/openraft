use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::error::Unreachable;
use openraft::AnyError;
use tokio::sync::oneshot;

use crate::app::RequestTx;
use crate::decode;
use crate::encode;
use crate::typ::RaftError;
use crate::NodeId;

/// Simulate a network router.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct Router {
    pub targets: Arc<Mutex<BTreeMap<NodeId, RequestTx>>>,
    /// Simulates a payload size limit. If set, requests exceeding this limit will fail.
    pub max_payload_size: Arc<Mutex<Option<usize>>>,
}

impl Router {
    /// Set the maximum payload size limit for simulating transport constraints.
    pub fn set_max_payload_size(&self, max_size: Option<usize>) {
        let mut limit = self.max_payload_size.lock().unwrap();
        *limit = max_size;
    }

    /// Send request `Req` to target node `to`, and wait for response `Result<Resp, RaftError<E>>`.
    pub async fn send<Req, Resp>(&self, to: NodeId, path: &str, req: &Req) -> Result<Resp, Unreachable>
    where
        Req: serde::Serialize,
        Result<Resp, RaftError>: serde::de::DeserializeOwned,
    {
        let (resp_tx, resp_rx) = oneshot::channel();

        let encoded_req = encode(req);
        tracing::debug!("send to: {}, {}, {}", to, path, encoded_req);

        {
            let mut targets = self.targets.lock().unwrap();
            let tx = targets.get_mut(&to).unwrap();

            tx.send((path.to_string(), encoded_req, resp_tx)).unwrap();
        }

        let resp_str = resp_rx.await.unwrap();
        tracing::debug!("resp from: {}, {}, {}", to, path, resp_str);

        // Empty response simulates transport-level error (e.g., HTTP 413 or gRPC RESOURCE_EXHAUSTED)
        if resp_str.is_empty() {
            return Err(Unreachable::new(&AnyError::error("payload too large")));
        }

        let res = decode::<Result<Resp, RaftError>>(&resp_str);
        res.map_err(|e| Unreachable::new(&e))
    }
}
