use anyhow::Result;
use openraft::Config;
use openraft::ConfigError;
use openraft::alias::MpscOf;
use openraft::async_runtime::Mpsc;
use openraft::errors::RaftError;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Sets one channel size of a `Config`.
type SetSize = fn(&mut Config, u64);

/// A channel size outside the runtime's `Mpsc::buffer_size_range()` makes `Raft::new` return
/// `ConfigError::ChannelSizeOutOfRange` naming the field, instead of panicking while it builds the
/// channel.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn unsupported_channel_size_fails_startup() -> Result<()> {
    // The Tokio-backed runtimes that these tests run on accept 1 to `Semaphore::MAX_PERMITS`, so
    // there is a rejected size on both sides of the range.
    let accepted = MpscOf::<TypeConfig>::buffer_size_range();
    let rejected = [accepted.start() - 1, accepted.end() + 1];

    let fields: [(&str, SetSize); 3] = [
        ("api_channel_size", |config, size| config.api_channel_size = Some(size)),
        ("notification_channel_size", |config, size| {
            config.notification_channel_size = Some(size)
        }),
        ("state_machine_channel_size", |config, size| {
            config.state_machine_channel_size = Some(size)
        }),
    ];

    for (field, set_size) in fields {
        for size in rejected {
            tracing::info!(
                field,
                size,
                ?accepted,
                "--- a size outside the accepted range fails Raft::new"
            );
            {
                let mut config = Config {
                    enable_tick: false,
                    enable_heartbeat: false,
                    enable_elect: false,
                    ..Default::default()
                };
                set_size(&mut config, u64::try_from(size)?);
                let mut router = RaftRouter::new(config)?;

                let started = router.new_raft_node(0).await;
                let error = started.unwrap_err();
                let expected = RaftError::APIError(ConfigError::ChannelSizeOutOfRange {
                    field,
                    size,
                    accepted: accepted.clone(),
                });
                let returned = error.downcast_ref::<RaftError<TypeConfig, ConfigError>>();
                assert_eq!(Some(&expected), returned, "{error:?}");
            }
        }

        tracing::info!(field, "--- a size of 1 starts and shuts down normally");
        {
            let mut config = Config {
                enable_tick: false,
                enable_heartbeat: false,
                enable_elect: false,
                ..Default::default()
            };
            set_size(&mut config, 1);
            let mut router = RaftRouter::new(config)?;

            router.new_raft_node(0).await?;
            router.stop_node(0).await?;
        }
    }

    Ok(())
}
