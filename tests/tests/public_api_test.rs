//! Compile-time test to verify all public API paths remain accessible from external crates.
//!
//! This test ensures that the extraction of async runtime traits into openraft-rt
//! does not break any existing public API paths.

#![allow(unused_imports)]
#![allow(dead_code)]

// Root-level exports from openraft
use openraft::AsyncRuntime;
use openraft::Instant;
use openraft::OptionalSend;
use openraft::OptionalSync;
use openraft::WatchSender;
use openraft::async_runtime;
// async_runtime module exports
use openraft::async_runtime::AsyncRuntime as AsyncRuntime2;
use openraft::async_runtime::Instant as Instant2;
use openraft::async_runtime::Mpsc;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::MpscSender;
use openraft::async_runtime::MpscWeakSender;
use openraft::async_runtime::Mutex;
use openraft::async_runtime::Oneshot;
use openraft::async_runtime::OneshotSender;
use openraft::async_runtime::RecvError;
use openraft::async_runtime::SendError;
use openraft::async_runtime::TokioInstant;
use openraft::async_runtime::TryRecvError;
use openraft::async_runtime::Watch;
use openraft::async_runtime::WatchReceiver;
use openraft::async_runtime::WatchSender as WatchSender2;
// Sub-module access
use openraft::async_runtime::instant;
// instant sub-module exports
use openraft::async_runtime::instant::Instant as InstantTrait;
use openraft::async_runtime::mpsc;
// mpsc sub-module exports
use openraft::async_runtime::mpsc::Mpsc as MpscTrait;
use openraft::async_runtime::mpsc::MpscReceiver as MpscReceiverTrait;
use openraft::async_runtime::mpsc::MpscSender as MpscSenderTrait;
use openraft::async_runtime::mpsc::MpscWeakSender as MpscWeakSenderTrait;
use openraft::async_runtime::mpsc::SendError as MpscSendError;
use openraft::async_runtime::mpsc::TryRecvError as MpscTryRecvError;
use openraft::async_runtime::mutex;
// mutex sub-module exports
use openraft::async_runtime::mutex::Mutex as MutexTrait;
use openraft::async_runtime::oneshot;
// oneshot sub-module exports
use openraft::async_runtime::oneshot::Oneshot as OneshotTrait;
use openraft::async_runtime::oneshot::OneshotSender as OneshotSenderTrait;
use openraft::async_runtime::watch;
// watch sub-module exports
use openraft::async_runtime::watch::RecvError as WatchRecvError;
use openraft::async_runtime::watch::SendError as WatchSendError;
use openraft::async_runtime::watch::Watch as WatchTrait;
use openraft::async_runtime::watch::WatchReceiver as WatchReceiverTrait;
use openraft::async_runtime::watch::WatchSender as WatchSenderTrait;

#[test]
fn test_public_api_accessible() {
    // This test just needs to compile to verify all paths are publicly accessible
}
