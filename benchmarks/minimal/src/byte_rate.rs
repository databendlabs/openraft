//! Shared byte pacing with bounded timer error and cancellation-safe queued work.

use std::future::Future;
use std::io;
use std::time::Duration;
use std::time::Instant;

pub(crate) struct ByteRate {
    cursor: tokio::sync::Mutex<Instant>,
}

struct Transfer<'a> {
    cursor: tokio::sync::MutexGuard<'a, Instant>,
    complete: bool,
}

impl Default for ByteRate {
    fn default() -> Self {
        Self {
            cursor: tokio::sync::Mutex::new(Instant::now()),
        }
    }
}

impl ByteRate {
    pub(crate) async fn wait<F, W>(&self, transfer: Duration, wait: F) -> io::Result<()>
    where
        F: FnOnce(Duration) -> W,
        W: Future<Output = io::Result<()>>,
    {
        if transfer.is_zero() {
            return Ok(());
        }
        let arrived = Instant::now();
        let cursor = self.cursor.lock().await;
        let finish = (*cursor).max(arrived) + transfer;
        let mut sending = Transfer {
            cursor,
            complete: false,
        };
        let remaining = finish.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            wait(remaining).await?;
        }
        *sending.cursor = finish;
        sending.complete = true;
        Ok(())
    }
}

impl Drop for Transfer<'_> {
    fn drop(&mut self) {
        if !self.complete {
            *self.cursor = Instant::now();
        }
    }
}
