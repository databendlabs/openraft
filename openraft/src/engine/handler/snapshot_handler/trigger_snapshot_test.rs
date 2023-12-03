use pretty_assertions::assert_eq;

use crate::core::sm;
use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng
}

#[test]
fn test_trigger_snapshot() -> anyhow::Result<()> {
    let mut eng = eng();

    assert_eq!(
        false,
        eng.state.io_state_mut().building_snapshot(),
        "initially, snapshot is not triggered"
    );

    // Trigger snapshot.

    let got = eng.snapshot_handler().trigger_snapshot();

    assert_eq!(true, got);
    assert_eq!(true, eng.state.io_state_mut().building_snapshot());
    assert_eq!(
        vec![
            //
            Command::from(sm::Command::build_snapshot().with_seq(1)),
        ],
        eng.output.take_commands()
    );

    // Trigger twice will not trigger again.

    let got = eng.snapshot_handler().trigger_snapshot();
    assert_eq!(false, got, "snapshot is already triggered");
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}
