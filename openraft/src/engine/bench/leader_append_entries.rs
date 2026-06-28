extern crate test;

use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use test::Bencher;
use test::black_box;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::payload::EntryPayload;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn setup_leader_engine() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false);

    eng.config.id = 1;
    eng.state.apply_progress_mut().accept(log_id(0, 1, 0));
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(1, 1, 1)),
            Membership::<u64, ()>::new_with_defaults(vec![btreeset! {0, 1}], []),
        )),
        Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(2, 1, 3)),
            Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2, 3}], btreeset! {1, 2, 3}),
        )),
    );
    eng.testing_new_leader();
    eng.state.server_state = eng.calc_server_state();
    eng.output.clear_commands();

    eng
}

#[bench]
fn leader_append_1_entry(b: &mut Bencher) {
    let mut eng = setup_leader_engine();
    let mut i = 0u64;

    b.iter(|| {
        eng.try_leader_handler().unwrap().leader_append_entries(black_box([EntryPayload::Blank]));

        i += 1;
        if i.is_multiple_of(64) {
            eng.output.clear_commands();
        }
    });
}

#[bench]
fn leader_append_3_entries(b: &mut Bencher) {
    let mut eng = setup_leader_engine();
    let mut i = 0u64;

    b.iter(|| {
        eng.try_leader_handler().unwrap().leader_append_entries(black_box([
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
        ]));

        i += 1;
        if i.is_multiple_of(64) {
            eng.output.clear_commands();
        }
    });
}

#[bench]
fn leader_append_10_entries(b: &mut Bencher) {
    let mut eng = setup_leader_engine();
    let mut i = 0u64;

    b.iter(|| {
        eng.try_leader_handler().unwrap().leader_append_entries(black_box([
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
            EntryPayload::Blank,
        ]));

        i += 1;
        if i.is_multiple_of(64) {
            eng.output.clear_commands();
        }
    });
}
