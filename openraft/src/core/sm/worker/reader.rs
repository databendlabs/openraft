use futures_util::StreamExt;

use super::work::Work;
use crate::RaftLogReader;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::MpscSenderOf;

/// The only owner of the log reader; a failed read stays in its ordered apply batch.
pub(super) struct Reader<C, SM, LR>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
    LR: RaftLogReader<C>,
{
    reader: LR,
    incoming: MpscReceiverOf<C, Work<C, SM>>,
    output: MpscSenderOf<C, Work<C, SM>>,
}

impl<C, SM, LR> Reader<C, SM, LR>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
    LR: RaftLogReader<C>,
{
    pub(super) fn new(
        reader: LR,
        incoming: MpscReceiverOf<C, Work<C, SM>>,
        output: MpscSenderOf<C, Work<C, SM>>,
    ) -> Self {
        Self {
            reader,
            incoming,
            output,
        }
    }

    pub(super) async fn run(self) -> Result<(), StorageError<C>> {
        let mut reader = self.reader;
        let mut incoming = self.incoming.peekable();
        while let Some(mut work) = incoming.recv().await {
            work.coalesce(&mut incoming);
            if let Work::Apply(batch) = &mut work {
                let since = batch.since();
                let end = batch.end();
                let stream = reader.entries_stream(since..end).await;
                batch.entries = stream.collect().await;
            }
            let sent = self.output.send(work).await;
            if sent.is_err() {
                return Ok(());
            }
        }
        Ok(())
    }
}
