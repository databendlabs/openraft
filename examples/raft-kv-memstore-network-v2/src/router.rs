use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::async_runtime::MpscSender;
use openraft::error::Unreachable;
use openraft::type_config::TypeConfigExt;

use crate::NodeId;
use crate::TypeConfig;
use crate::app::RequestTx;
use crate::decode;
use crate::encode;
use crate::typ::RaftError;

/// Simulate a network router.
#[derive(Clone, Default)]
pub struct Router {
    pub targets: Arc<Mutex<BTreeMap<NodeId, RequestTx>>>,
}

impl Router {
    /// Send request `Req` to target node `to`, and wait for response `Result<Resp, RaftError<E>>`.
    pub async fn send<Req, Resp>(&self, to: NodeId, path: &str, req: Req) -> Result<Resp, Unreachable<TypeConfig>>
    where
        Req: serde::Serialize,
        Result<Resp, RaftError>: serde::de::DeserializeOwned,
    {
        let (resp_tx, resp_rx) = TypeConfig::oneshot();

        let encoded_req = encode(req);
        tracing::debug!("send to: {}, {}, {}", to, path, encoded_req);

        let tx = {
            let targets = self.targets.lock().unwrap();
            targets.get(&to).unwrap().clone()
        };

        MpscSender::send(&tx, (path.to_string(), encoded_req, resp_tx)).await.unwrap();

        let resp_str = resp_rx.await.unwrap();
        tracing::debug!("resp from: {}, {}, {}", to, path, resp_str);

        let res = decode::<Result<Resp, RaftError>>(&resp_str);
        res.map_err(|e| Unreachable::new(&e))
    }
}
