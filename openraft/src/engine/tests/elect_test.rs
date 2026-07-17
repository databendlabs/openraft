use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft_rt::deterministic_rng::DeterministicRng;
use openraft_rt_tokio::TokioRuntime;
use pretty_assertions::assert_eq;
use rand::RngExt;

use crate::AsyncRuntime;
use crate::Config;
use crate::Membership;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::EngineConfig;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::vote::RaftLeaderIdExt;

fn m1() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1}], [])
}

fn m12() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.log_ids = LogIdList::new(None, [log_id(0, 0, 0)]);
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng
}

type SeededRuntime = DeterministicRng<TokioRuntime>;

crate::declare_raft_types!(
    SeededConfig:
        D = u64,
        R = (),
        Node = (),
        AsyncRuntime = SeededRuntime,
);

fn run_seeded<F, T>(seed: u64, test: F) -> T
where
    F: FnOnce() -> T,
    T: Send,
{
    let mut runtime = TokioRuntime::new(1);
    runtime.block_on(SeededRuntime::scope(seed, async move { test() }))
}

fn seeded_log_id(term: u64, node_id: u64, index: u64) -> LogIdOf<SeededConfig> {
    LogIdOf::<SeededConfig>::new(LeaderIdOf::<SeededConfig>::new_committed(term, node_id), index)
}

fn seeded_engine(id: u64, config: EngineConfig<SeededConfig>) -> Engine<SeededConfig> {
    let mut engine = Engine::<SeededConfig>::testing_default(id);
    engine.config = config;
    engine.state.log_ids = LogIdList::new(None, [seeded_log_id(0, 0, 0)]);
    engine.state.enable_validation(false);
    engine.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<SeededConfig>::new(
        Some(seeded_log_id(0, 1, 1)),
        m12(),
    )));
    engine
}

fn election_config() -> Config {
    Config {
        election_timeout_min: 100,
        election_timeout_max: 108,
        ..Default::default()
    }
    .validate()
    .expect("valid election timing bounds")
}

fn reference_timeouts(seed: u64, count: usize) -> Vec<Duration> {
    let config = election_config();
    run_seeded(seed, move || {
        (0..count)
            .map(|_| {
                let millis =
                    SeededRuntime::thread_rng().random_range(config.election_timeout_min..config.election_timeout_max);
                Duration::from_millis(millis)
            })
            .collect()
    })
}

#[test]
fn test_default_election_timer_bounds_are_coherent() {
    let config = EngineConfig::<UTConfig>::new_default(1);
    let minimum = Duration::from_millis(config.election_timeout_min);
    let maximum = Duration::from_millis(config.election_timeout_max);

    assert!(minimum <= config.timer_config.election_timeout);
    assert!(config.timer_config.election_timeout < maximum);
    assert_eq!(maximum, config.timer_config.leader_lease);
    assert_eq!(maximum * 2, config.timer_config.smaller_log_timeout);
}

#[test]
fn test_direct_transfer_and_pre_vote_campaigns_resample_timeout() {
    const SEED: u64 = 44;
    let expected = reference_timeouts(SEED, 4);
    assert_eq!(4, expected.iter().collect::<BTreeSet<_>>().len(), "{expected:?}");

    let observed = run_seeded(SEED, || {
        let config = EngineConfig::new(1, &election_config());
        let leader_lease = config.timer_config.leader_lease;
        let smaller_log_timeout = config.timer_config.smaller_log_timeout;
        let mut observed = vec![config.timer_config.election_timeout];
        let mut engine = seeded_engine(1, config);

        engine.elect();
        observed.push(engine.config.timer_config.election_timeout);

        engine.elect_by_leadership_transfer();
        observed.push(engine.config.timer_config.election_timeout);

        engine.pre_elect();
        observed.push(engine.config.timer_config.election_timeout);

        assert_eq!(Vote::new(2, 1), *engine.state.vote_ref());
        assert_eq!(leader_lease, engine.config.timer_config.leader_lease);
        assert_eq!(smaller_log_timeout, engine.config.timer_config.smaller_log_timeout);
        observed
    });

    assert_eq!(expected, observed);
}

#[test]
fn test_tied_initial_samples_can_diverge_after_resampling() {
    const SEED_ONE: u64 = 0;
    const SEED_TWO: u64 = 5;
    let expected_one = reference_timeouts(SEED_ONE, 3);
    let expected_two = reference_timeouts(SEED_TWO, 3);
    assert_eq!(expected_one[0], expected_two[0]);
    assert_ne!(expected_one[1..], expected_two[1..]);

    let observe = |seed, id| {
        run_seeded(seed, || {
            let config = EngineConfig::new(id, &election_config());
            let mut observed = vec![config.timer_config.election_timeout];
            let mut engine = seeded_engine(id, config);
            engine.pre_elect();
            observed.push(engine.config.timer_config.election_timeout);
            engine.pre_elect();
            observed.push(engine.config.timer_config.election_timeout);
            observed
        })
    };

    let observed_one = observe(SEED_ONE, 1);
    let observed_two = observe(SEED_TWO, 2);
    assert_eq!(expected_one, observed_one);
    assert_eq!(expected_two, observed_two);
    assert_eq!(observed_one[0], observed_two[0]);

    // Fresh independent draws permit tied nodes to diverge; they do not
    // guarantee that two nodes can never sample the same timeout again.
    assert_ne!(observed_one[1..], observed_two[1..]);
}

#[test]
fn test_successful_pre_vote_resamples_the_real_campaign() {
    const SEED: u64 = 44;
    let expected = reference_timeouts(SEED, 3);
    assert_eq!(3, expected.iter().collect::<BTreeSet<_>>().len(), "{expected:?}");

    let observed = run_seeded(SEED, || {
        let config = EngineConfig::new(1, &election_config());
        let initial = config.timer_config.election_timeout;
        let mut engine = seeded_engine(1, config);
        engine.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<SeededConfig>::new(
            Some(seeded_log_id(0, 1, 1)),
            m1(),
        )));

        engine.pre_elect();

        assert_eq!(Vote::new(1, 1), *engine.state.vote_ref());
        [initial, engine.config.timer_config.election_timeout]
    });

    assert_eq!(expected[0], observed[0]);
    assert_eq!(expected[2], observed[1]);
}

#[test]
fn test_elect_single_node() -> anyhow::Result<()> {
    tracing::info!("--- single node: still need to wait for Vote to persist");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(0, 1, 1)),
            m1(),
        )));

        eng.elect();

        assert_eq!(Vote::new(1, 1), *eng.state.vote_ref());
        assert!(eng.leader.is_none());
        assert!(eng.candidate_ref().is_some(), "candidate state is pending");

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(1, 1) },
                Command::SendVote {
                    vote_req: VoteRequest {
                        vote: Vote::new(1, 1),
                        last_log_id: Some(log_id(0, 0, 0)),
                        leadership_transfer: false,
                    },
                },
            ],
            eng.output.take_commands()
        );
    }

    Ok(())
}

#[test]
fn test_elect_by_leadership_transfer_sets_flag() -> anyhow::Result<()> {
    // An election started by a leadership transfer marks the vote request, so that voters
    // grant it even when the leader lease has not expired.
    let mut eng = eng();
    eng.config.id = 1;
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(0, 1, 1)),
        m12(),
    )));

    eng.elect_by_leadership_transfer();

    assert_eq!(Vote::new(1, 1), *eng.state.vote_ref());
    assert!(eng.candidate_ref().is_some(), "candidate state is pending");
    assert_eq!(ServerState::Candidate, eng.state.server_state);

    assert_eq!(
        vec![
            //
            Command::SaveVote { vote: Vote::new(1, 1) },
            Command::SendVote {
                vote_req: VoteRequest {
                    vote: Vote::new(1, 1),
                    last_log_id: Some(log_id(0, 0, 0)),
                    leadership_transfer: true,
                },
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_elect_again_overrides_previous_campaign() -> anyhow::Result<()> {
    tracing::info!("--- electing again bumps the term off the current vote and overrides the previous campaign");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(0, 1, 1)),
            m1(),
        )));

        // The first campaign, to be overridden by the second one.
        eng.elect();
        assert_eq!(Vote::new(1, 1), *eng.state.vote_ref());
        assert_eq!(Vote::new(1, 1), *eng.candidate_ref().unwrap().vote_ref());
        eng.output.take_commands();

        eng.elect();

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(Vote::new(2, 1), *eng.candidate_ref().unwrap().vote_ref());

        assert!(eng.candidate_mut().is_some(), "candidate state is pending");

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(2, 1) },
                Command::SendVote {
                    vote_req: VoteRequest {
                        vote: Vote::new(2, 1),
                        last_log_id: Some(log_id(0, 0, 0)),
                        leadership_transfer: false,
                    },
                },
            ],
            eng.output.take_commands()
        );
    }
    Ok(())
}

#[test]
fn test_election_on_leader_is_ignored() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.config.id = 1;
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(0, 1, 1)),
        m12(),
    )));

    eng.elect();
    eng.candidate_mut().unwrap().grant_by(&1);
    eng.handle_vote_resp(2, VoteResponse::new(Vote::new(1, 1), Some(log_id(0, 0, 0)), true));
    eng.output.take_commands();

    eng.new_pre_candidate(Vote::new(2, 1));
    assert_eq!(Vote::new(2, 1), *eng.pre_candidate_ref().unwrap().vote_ref());

    eng.elect();
    eng.elect_by_leadership_transfer();

    assert_eq!(Vote::new_committed(1, 1), *eng.state.vote_ref());
    assert!(eng.leader.is_some(), "leadership is preserved");
    assert!(eng.candidate_ref().is_none(), "no campaign is created");
    assert!(eng.pre_candidate_ref().is_none(), "no pre-vote is retained");
    assert_eq!(ServerState::Leader, eng.state.server_state);
    assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());

    Ok(())
}

#[test]
fn test_elect_multi_node_enter_candidate() -> anyhow::Result<()> {
    tracing::info!("--- multi nodes: enter candidate state");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(0, 1, 1)),
            m12(),
        )));
        eng.state.log_ids = LogIdList::new(None, vec![log_id(1, 1, 1)]);

        eng.elect();

        assert_eq!(Vote::new(1, 1), *eng.state.vote_ref());
        assert!(eng.leader.is_none());
        assert_eq!(Vote::new(1, 1), *eng.candidate_ref().unwrap().vote_ref());

        assert_eq!(
            Some(btreeset! {},),
            eng.candidate_ref().map(|x| x.granters().collect::<BTreeSet<_>>())
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(1, 1) },
                Command::SendVote {
                    vote_req: VoteRequest::new(Vote::new(1, 1), Some(log_id(1, 1, 1)))
                },
            ],
            eng.output.take_commands()
        );
    }
    Ok(())
}
