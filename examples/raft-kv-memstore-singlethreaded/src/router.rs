use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Mutex;

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
    pub targets: Rc<Mutex<BTreeMap<NodeId, RequestTx>>>,
}

impl Router {
    /// Send request `Req` to target node `to`, and wait for response `Result<Resp, RaftError<E>>`.
    pub async fn send<Req, Resp, E>(&self, to: NodeId, path: &str, req: Req) -> Result<Resp, RaftError<E>>
    where
        Req: serde::Serialize,
        Result<Resp, RaftError<E>>: serde::de::DeserializeOwned,
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

        decode::<Result<Resp, RaftError<E>>>(&resp_str)
    }
}
