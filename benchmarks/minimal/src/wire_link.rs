//! Serialized, bounded directed links shared by every RPC using the same node pair.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;

use crate::byte_rate::ByteRate;
use crate::profile::Profile;
use crate::profile::transfer_time;

pub(crate) struct WireLink {
    profile: Arc<Profile>,
    delay: Duration,
    capacity: Arc<Semaphore>,
    state: Mutex<State>,
    transfer: ByteRate,
}

struct State {
    messages: u64,
    bytes: u64,
    seed: u64,
    peak: usize,
    encoded_bytes: usize,
    peak_encoded_bytes: usize,
    delivered: u64,
    cancelled: u64,
    delays: VecDeque<(u64, u64)>,
}

struct EncodedBuffer<'a> {
    state: &'a Mutex<State>,
    bytes: Vec<u8>,
    completed: bool,
}

impl Drop for EncodedBuffer<'_> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        state.encoded_bytes -= self.bytes.capacity();
        if !self.completed {
            state.cancelled += 1;
        }
    }
}

impl WireLink {
    pub(crate) fn new(profile: Arc<Profile>, delay: Duration, source: u64, target: u64) -> Self {
        Self {
            capacity: Arc::new(Semaphore::new(profile.link_capacity)),
            profile,
            delay,
            state: Mutex::new(State {
                messages: 0,
                bytes: 0,
                seed: source.wrapping_mul(65537) ^ target,
                peak: 0,
                encoded_bytes: 0,
                peak_encoded_bytes: 0,
                delivered: 0,
                cancelled: 0,
                delays: VecDeque::new(),
            }),
            transfer: ByteRate::default(),
        }
    }

    pub(crate) async fn transmit<T>(&self, value: T) -> io::Result<T>
    where T: Serialize + DeserializeOwned {
        let started = Instant::now();
        let permit = self.capacity.acquire().await.map_err(io::Error::other)?;
        let bytes = serde_json::to_vec(&value).map_err(io::Error::other)?;
        drop(value);
        let delay = self.reserve(bytes.len(), bytes.capacity());
        let transfer = transfer_time(bytes.len(), self.profile.link_bytes_per_second);
        let mut buffer = EncodedBuffer {
            state: &self.state,
            bytes,
            completed: false,
        };
        self.transfer
            .wait(transfer, |duration| async move {
                tokio::time::sleep(duration).await;
                Ok(())
            })
            .await?;
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let result = serde_json::from_slice(&buffer.bytes).map_err(io::Error::other);
        {
            let mut state = self.state.lock().unwrap();
            state.delivered += 1;
            if state.delays.len() == 4096 {
                state.delays.pop_front();
            }
            state.delays.push_back((
                (delay + transfer).as_nanos() as u64,
                started.elapsed().as_nanos() as u64,
            ));
        }
        buffer.completed = true;
        drop(buffer);
        drop(permit);
        result
    }

    fn reserve(&self, bytes: usize, allocated: usize) -> Duration {
        let mut state = self.state.lock().unwrap();
        state.seed = state.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let jitter = state.seed % self.profile.network_jitter_us.saturating_add(1);
        state.messages += 1;
        state.bytes += bytes as u64;
        let occupied = self.profile.link_capacity - self.capacity.available_permits();
        state.peak = state.peak.max(occupied);
        state.encoded_bytes += allocated;
        state.peak_encoded_bytes = state.peak_encoded_bytes.max(state.encoded_bytes);
        self.delay + Duration::from_micros(jitter)
    }

    pub(crate) fn report(&self) -> serde_json::Value {
        let state = self.state.lock().unwrap();
        serde_json::json!({"messages":state.messages,"serialized_bytes":state.bytes,
            "peak_admitted":state.peak,"capacity":self.profile.link_capacity,
            "current_encoded_buffer_bytes":state.encoded_bytes,"peak_encoded_buffer_bytes":state.peak_encoded_bytes,
            "memory_scope":"encoded_Vec_capacities_excludes_original_decoded_and_waiting_payloads",
            "delivered":state.delivered,"cancelled_admitted":state.cancelled,
            "requested_and_enqueue_to_decode_ns":state.delays,"delay_sample_scope":"latest_4096_deliveries_including_admission_serialization_and_timer",
            "bandwidth_scope":"all_rpc_messages_on_directed_link","timer":"tokio_millisecond"})
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::Instant;

    use super::WireLink;
    use crate::profile::Profile;

    #[tokio::test]
    #[cfg(feature = "realistic-profiles")]
    async fn wire_copy_does_not_share_follower_payloads() -> anyhow::Result<()> {
        let profile = Arc::new(Profile {
            profile: true,
            payload_bytes: 256,
            serialized_transport: true,
            ..Profile::default()
        });
        let original = profile.request(3)?;
        let request = original.clone();
        let link = WireLink::new(profile, Duration::ZERO, 0, 1);
        let decoded = link.transmit(request).await?;
        assert_eq!(original, decoded);
        let shared = Arc::ptr_eq(original.payload.as_ref().unwrap(), decoded.payload.as_ref().unwrap());
        assert!(!shared);
        Ok(())
    }

    #[tokio::test]
    async fn bandwidth_and_capacity_are_shared_by_concurrent_messages() -> anyhow::Result<()> {
        let profile = Arc::new(Profile {
            profile: true,
            serialized_transport: true,
            link_capacity: 2,
            link_bytes_per_second: 1000,
            ..Profile::default()
        });
        let link = WireLink::new(profile, Duration::ZERO, 0, 1);
        let value = "a".repeat(100);
        let started = Instant::now();
        let (first, second) = tokio::join!(link.transmit(value.clone()), link.transmit(value.clone()));
        assert_eq!(value, first?);
        assert_eq!(value, second?);
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(204));
        let report = link.report();
        assert_eq!(204, report["serialized_bytes"]);
        assert_eq!(2, report["peak_admitted"]);
        Ok(())
    }

    #[tokio::test]
    async fn queued_small_messages_keep_timer_oversleep_credit() -> anyhow::Result<()> {
        let profile = Arc::new(Profile {
            profile: true,
            serialized_transport: true,
            link_capacity: 128,
            link_bytes_per_second: 1_000_000,
            ..Profile::default()
        });
        let link = WireLink::new(profile, Duration::ZERO, 0, 1);
        let started = Instant::now();
        let messages = (0..128).map(|_| link.transmit(1u64));
        let results = futures::future::try_join_all(messages).await?;
        assert_eq!(vec![1; 128], results);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "small packets paid a timer tick each: {elapsed:?}"
        );
        let report = link.report();
        assert_eq!(128, report["serialized_bytes"]);
        Ok(())
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_delivery_releases_link_capacity() -> anyhow::Result<()> {
        let profile = Arc::new(Profile {
            profile: true,
            serialized_transport: true,
            link_capacity: 1,
            ..Profile::default()
        });
        let link = Arc::new(WireLink::new(profile, Duration::from_secs(1), 0, 1));
        let sending = link.clone();
        let handle = tokio::spawn(async move { sending.transmit(1u64).await });
        loop {
            if link.capacity.available_permits() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        handle.abort();
        let result = handle.await;
        let error = result.unwrap_err();
        assert!(error.is_cancelled());
        let available = link.capacity.available_permits();
        assert_eq!(1, available);
        let report = link.report();
        assert_eq!(1, report["peak_admitted"]);
        assert_eq!(0, report["current_encoded_buffer_bytes"]);
        let peak = report["peak_encoded_buffer_bytes"].as_u64().unwrap();
        assert!(peak > 0);
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_large_transfer_does_not_reserve_unsent_bandwidth() -> anyhow::Result<()> {
        let profile = Arc::new(Profile {
            profile: true,
            serialized_transport: true,
            link_bytes_per_second: 1024,
            ..Profile::default()
        });
        let link = Arc::new(WireLink::new(profile, Duration::ZERO, 0, 1));
        let sending = link.clone();
        let handle = tokio::spawn(async move { sending.transmit("a".repeat(65536)).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while link.report()["messages"] == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        handle.abort();
        let result = handle.await;
        let error = result.unwrap_err();
        assert!(error.is_cancelled());
        let small = tokio::time::timeout(Duration::from_millis(100), link.transmit(1u64)).await??;
        assert_eq!(1, small);
        let report = link.report();
        assert_eq!(1, report["cancelled_admitted"]);
        assert_eq!(1, report["delivered"]);
        assert_eq!(0, report["current_encoded_buffer_bytes"]);
        Ok(())
    }
}
