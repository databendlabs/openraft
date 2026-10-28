use std::env;
use std::time::Duration;

use openraft::AsyncRuntime;
use openraft::RaftTypeConfig;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::AsyncRuntimeOf;
use rand::RngExt;

/// The delay in milliseconds from one `OPENRAFT_NETWORK_SEND_DELAY` value.
///
/// `None` keeps `fallback_ms`. A decimal string replaces it.
pub fn delay_from_env(raw: Option<&str>, fallback_ms: u64) -> u64 {
    let Some(raw) = raw else {
        return fallback_ms;
    };
    raw.parse::<u64>().unwrap()
}

/// Read `OPENRAFT_NETWORK_SEND_DELAY`, or keep `fallback_ms` when it is absent.
pub(super) fn configured_network_send_delay(fallback_ms: u64) -> u64 {
    let raw = env::var("OPENRAFT_NETWORK_SEND_DELAY").ok();
    if let Some(ref value) = raw {
        tracing::info!("OPENRAFT_NETWORK_SEND_DELAY set send-delay to {} ms", value);
    }
    delay_from_env(raw.as_deref(), fallback_ms)
}

/// Sleep for one send delay on `C`'s runtime.
///
/// A `delay_ms` of 0 returns immediately. A positive delay sleeps for a random
/// duration of at least 1 ms and at most `delay_ms`. The draw is
/// `random % delay_ms`, and that remainder can be 0. Sleeping for 0 ms makes an
/// observation of the delay flake, so the sleep adds 1 ms.
pub(super) async fn apply_send_delay<C>(delay_ms: u64)
where C: RaftTypeConfig {
    if delay_ms == 0 {
        return;
    }

    let random = AsyncRuntimeOf::<C>::thread_rng().random::<u64>();
    let offset = random % delay_ms;
    let sleep_ms = offset + 1;
    let timeout = Duration::from_millis(sleep_ms);
    C::sleep(timeout).await;
}
