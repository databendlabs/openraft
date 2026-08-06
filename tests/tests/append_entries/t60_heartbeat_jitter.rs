use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use openraft::impls::TokioRuntime;
use openraft::type_config::AsyncRuntime;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::ut_harness;

/// Reproduce the p99 latency impact of tick synchronization in multiraft.
///
/// Simulates 128 heartbeat handlers (64 groups × 2 followers) triggered either
/// all at once (no jitter — thundering herd) or spread across the tick interval
/// (with jitter). Each handler does simulated RPC work. Measures per-handler
/// latency and compares p99 between the two modes.
///
/// Without jitter: all 128 handlers compete for tokio poll slots in the same
/// cycle, causing queueing — later handlers wait for earlier ones to yield.
/// With jitter: handlers rarely overlap, so each completes without waiting.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn heartbeat_jitter_prevents_clustering() -> Result<()> {
    let n_handlers: usize = 128; // 64 groups × 2 followers
    let interval = Duration::from_millis(100);

    // Simulates the work each append_entries handler does:
    // yield to runtime + small CPU work + async I/O wait.
    async fn simulate_rpc_work() {
        TypeConfig::yield_now().await;
        // Simulate serialization / log-append CPU cost
        let mut sum = 0u64;
        for i in 0..10_000 {
            sum = sum.wrapping_add(i);
        }
        std::hint::black_box(sum);
        // Simulate fsync / network response wait
        TokioRuntime::sleep(Duration::from_micros(500)).await;
    }

    // --- No jitter: all handlers fire at the same instant ---
    let no_jitter_latencies = {
        let latencies: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));

        // Wait for one interval so the timer wheel settles, then fire all at once
        TokioRuntime::sleep(interval).await;

        let mut handles = Vec::with_capacity(n_handlers);

        for _ in 0..n_handlers {
            let lat = latencies.clone();
            handles.push(TypeConfig::spawn(async move {
                let start = Instant::now();
                simulate_rpc_work().await;
                let elapsed = start.elapsed();
                lat.lock().unwrap().push(elapsed);
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let mut lats = latencies.lock().unwrap().clone();
        lats.sort();
        lats
    };

    // --- With jitter: handlers spread across [0, interval) ---
    let jitter_latencies = {
        let latencies: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));

        let mut handles = Vec::with_capacity(n_handlers);

        for i in 0..n_handlers {
            let lat = latencies.clone();
            // Deterministic spread: handler i fires at i * interval / n_handlers
            let delay = Duration::from_micros((i as u64 * interval.as_micros() as u64) / n_handlers as u64);
            handles.push(TypeConfig::spawn(async move {
                TokioRuntime::sleep(delay).await;
                let start = Instant::now();
                simulate_rpc_work().await;
                let elapsed = start.elapsed();
                lat.lock().unwrap().push(elapsed);
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let mut lats = latencies.lock().unwrap().clone();
        lats.sort();
        lats
    };

    // Compute percentiles
    let percentile = |lats: &[Duration], p: f64| -> Duration {
        let idx = ((lats.len() as f64 * p) as usize).min(lats.len() - 1);
        lats[idx]
    };

    let no_jitter_p50 = percentile(&no_jitter_latencies, 0.50);
    let no_jitter_p99 = percentile(&no_jitter_latencies, 0.99);
    let no_jitter_max = *no_jitter_latencies.last().unwrap();

    let jitter_p50 = percentile(&jitter_latencies, 0.50);
    let jitter_p99 = percentile(&jitter_latencies, 0.99);
    let jitter_max = *jitter_latencies.last().unwrap();

    tracing::info!(
        "no_jitter: n={}, p50={:.2}ms, p99={:.2}ms, max={:.2}ms",
        no_jitter_latencies.len(),
        no_jitter_p50.as_secs_f64() * 1000.0,
        no_jitter_p99.as_secs_f64() * 1000.0,
        no_jitter_max.as_secs_f64() * 1000.0,
    );
    tracing::info!(
        "   jitter: n={}, p50={:.2}ms, p99={:.2}ms, max={:.2}ms",
        jitter_latencies.len(),
        jitter_p50.as_secs_f64() * 1000.0,
        jitter_p99.as_secs_f64() * 1000.0,
        jitter_max.as_secs_f64() * 1000.0,
    );

    // With jitter, p99 should be lower because handlers don't queue behind each other.
    // The no-jitter case has all 128 handlers contending simultaneously.
    assert!(
        jitter_p99 < no_jitter_p99,
        "Jitter should reduce p99 latency: jitter_p99={:.2}ms < no_jitter_p99={:.2}ms",
        jitter_p99.as_secs_f64() * 1000.0,
        no_jitter_p99.as_secs_f64() * 1000.0,
    );

    Ok(())
}
