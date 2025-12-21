use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::TryRecvError;
use crate::core::raft_msg::RaftMsg;
use crate::error::Fatal;
use crate::type_config::alias::MpscReceiverOf;

pub(crate) struct BatchRaftMsgReceiver<C>
where C: RaftTypeConfig
{
    buffered: Option<RaftMsg<C>>,
    inner: MpscReceiverOf<C, RaftMsg<C>>,
}

impl<C> BatchRaftMsgReceiver<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(receiver: MpscReceiverOf<C, RaftMsg<C>>) -> Self {
        Self {
            buffered: None,
            inner: receiver,
        }
    }

    pub(crate) async fn ensure_buffered(&mut self) -> Result<(), Fatal<C>> {
        if self.buffered.is_some() {
            return Ok(());
        }

        let msg = self.inner_recv().await?;
        self.buffered = Some(msg);

        Ok(())
    }

    pub(crate) fn try_recv(&mut self) -> Result<Option<RaftMsg<C>>, Fatal<C>> {
        let msg = self.buffered_try_recv()?;

        let Some(mut msg) = msg else {
            return Ok(None);
        };

        self.merge_client_writes(&mut msg)?;

        Ok(Some(msg))
    }

    fn buffered_try_recv(&mut self) -> Result<Option<RaftMsg<C>>, Fatal<C>> {
        if let Some(msg) = self.buffered.take() {
            return Ok(Some(msg));
        }

        self.inner_try_recv()
    }

    async fn inner_recv(&mut self) -> Result<RaftMsg<C>, Fatal<C>> {
        let Some(msg) = self.inner.recv().await else {
            tracing::info!("all rx_api senders are dropped");
            return Err(Fatal::Stopped);
        };

        Ok(msg)
    }

    fn inner_try_recv(&mut self) -> Result<Option<RaftMsg<C>>, Fatal<C>> {
        let res = self.inner.try_recv();

        match res {
            Ok(msg) => Ok(Some(msg)),
            Err(e) => match e {
                TryRecvError::Empty => {
                    tracing::debug!("all RaftMsg are processed, wait for more");
                    Ok(None)
                }
                TryRecvError::Disconnected => {
                    tracing::debug!("rx_api is disconnected, quit");
                    Err(Fatal::Stopped)
                }
            },
        }
    }

    /// Merge as many consecutive RaftMsg::ClientWrite as possible, if they have the same
    /// expected_leader arg.
    ///
    /// When a unmergable RaftMsg is found, it saves it in self.next_raft_msg for next polling
    ///
    /// This method does not check `next`, and may leave a unmerged RaftMsg in `next`.
    fn merge_client_writes(&mut self, msg: &mut RaftMsg<C>) -> Result<(), Fatal<C>> {
        debug_assert!(self.buffered.is_none());

        let (batch_data, batch_responders, batch_leader) = match msg {
            RaftMsg::ClientWrite {
                app_data,
                responders,
                expected_leader,
            } => (app_data, responders, expected_leader),
            _ => return Ok(()),
        };

        // TODO: make it configurable
        let max_batch_size = 4096;

        for _i in 0..max_batch_size {
            let next = self.inner_try_recv()?;

            let Some(next) = next else {
                break;
            };

            let RaftMsg::ClientWrite {
                app_data,
                responders,
                expected_leader,
            } = next
            else {
                self.buffered = Some(next);
                break;
            };

            if &expected_leader == batch_leader {
                batch_data.extend(app_data);
                batch_responders.extend(responders);
            } else {
                self.buffered = Some(RaftMsg::ClientWrite {
                    app_data,
                    responders,
                    expected_leader,
                });
                break;
            }
        }

        Ok(())
    }
}
