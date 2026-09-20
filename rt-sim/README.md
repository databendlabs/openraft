# openraft-rt-sim

A deterministic, simulated [`AsyncRuntime`][rt_link] for testing Openraft.

- One thread, FIFO scheduling: tasks are polled in the order they became runnable.
- Virtual time: when nothing is runnable, the clock jumps to the earliest pending timer.
  Timers with equal deadlines fire in registration order.
- Seeded `thread_rng()`: every call draws from a stream derived from the runtime seed.
- If nothing is runnable and no timer is pending, `block_on` panics instead of hanging.

With the `sim-log` feature and `OPENRAFT_SIM_LOG=<file>`, `block_on` writes the tracing events of
its thread to that file, stamped with virtual time: the same seed writes the same log.

See [openraft#206](https://github.com/databendlabs/openraft/issues/206).

[rt_link]: https://docs.rs/openraft/latest/openraft/async_runtime/trait.AsyncRuntime.html
