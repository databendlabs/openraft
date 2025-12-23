use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use openraft::error::RemoteError;
use openraft::error::Unreachable;
use tokio::sync::oneshot;

use crate::app::RequestTx;
use crate::decode;
use crate::encode;
use crate::typ::RPCError;
use crate::typ::RaftError;
use crate::NodeId;

/// Simulate a network router.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct Router {
    pub targets: Rc<RefCell<BTreeMap<NodeId, RequestTx>>>,
}

impl Router {
    /// Send request `Req` to target node `to`, and wait for response `Result<Resp, E>`.
    pub async fn send<Req, Resp, E>(&self, to: NodeId, path: &str, req: Req) -> Result<Resp, RPCError<E>>
    where
        Req: serde::Serialize,
        Result<Resp, RaftError<E>>: serde::de::DeserializeOwned,
        E: std::error::Error,
    {
        let (resp_tx, resp_rx) = oneshot::channel();

        let encoded_req = encode(req);
        tracing::debug!("send to: {}, {}, {}", to, path, encoded_req);

        {
            let mut targets = self.targets.borrow_mut();
            let tx = targets.get_mut(&to).unwrap();

            tx.send((path.to_string(), encoded_req, resp_tx)).unwrap();
        }

        let resp_str = resp_rx.await.unwrap();
        tracing::debug!("resp from: {}, {}, {}", to, path, resp_str);

        let res = decode::<Result<Resp, RaftError<E>>>(&resp_str);
        match res {
            Ok(r) => Ok(r),
            Err(e) => match e {
                RaftError::APIError(x) => Err(RPCError::RemoteError(RemoteError::new(to, x))),
                RaftError::Fatal(f) => Err(RPCError::Unreachable(Unreachable::new(&f))),
            },
        }
    }
}
