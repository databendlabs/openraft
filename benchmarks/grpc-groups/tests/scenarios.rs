//! Run each scenario briefly with a few groups, and check the fields that the run prints.

use std::collections::BTreeMap;
use std::process::Command;

/// The fields of one line that a run printed, by name.
type Fields = BTreeMap<String, String>;

/// Run the benchmark with `args`, and return the fields of each line it printed.
fn run(args: &[&str]) -> Vec<Fields> {
    let output = Command::new(env!("CARGO_BIN_EXE_grpc-groups"))
        .arg("run")
        .args(args)
        .output()
        .expect("run the benchmark");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the run failed, stderr: {stderr}");

    let stdout = String::from_utf8(output.stdout).expect("the output is UTF-8");
    let parse_line = |line: &str| {
        let fields = line.split('\t').map(|field| field.split_once('=').expect("a field is name=value"));
        fields.map(|(name, value)| (name.to_string(), value.to_string())).collect::<Fields>()
    };
    stdout.lines().map(parse_line).collect()
}

fn number(fields: &Fields, name: &str) -> f64 {
    let value = &fields[name];
    value.parse().unwrap_or_else(|_| panic!("{name}={value} is not a number, in: {fields:?}"))
}

#[test]
fn test_steady_measures_the_heartbeats_between_the_nodes() {
    let lines = run(&[
        "--scenario",
        "steady",
        "--groups",
        "6",
        "--settle-secs",
        "1",
        "--measure-secs",
        "2",
    ]);

    assert_eq!(1, lines.len(), "got: {lines:?}");
    let measure = &lines[0];
    assert_eq!("steady", measure["scenario"]);
    assert_eq!("measure", measure["phase"]);
    assert!(number(measure, "appends/s") > 0.0, "got: {measure:?}");
    assert_eq!(
        measure["appends/s"], measure["rpcs/s"],
        "without batching, each request is one call"
    );
    assert!(number(measure, "hb_rtt_p50_us") > 0.0, "got: {measure:?}");
    assert!(number(measure, "ack_age_p50_ms") > 0.0, "got: {measure:?}");
    assert!(number(measure, "tasks") > 0.0, "got: {measure:?}");
    assert_eq!("0", measure["elections"], "every group keeps its leader");
}

#[test]
fn test_crash_measures_the_failover_of_the_groups_node_1_led() {
    let lines = run(&[
        "--scenario",
        "crash",
        "--groups",
        "6",
        "--settle-secs",
        "1",
        "--window-secs",
        "2",
    ]);

    let phases = lines.iter().map(|line| line["phase"].as_str()).collect::<Vec<_>>();
    assert_eq!(vec!["baseline", "failover", "degraded"], phases);

    let baseline = &lines[0];
    assert_eq!("0", baseline["elections"], "every group keeps its leader");

    // Node 1 led groups 0 and 3 of the 6.
    let failover = &lines[1];
    assert_eq!("2", failover["affected"]);
    assert_eq!("0", failover["unrecovered"]);
    assert_eq!("0", failover["moved"], "the other groups keep their leader");
    // A follower grants no vote until the lease of the leader it last heard from expires, 300 ms
    // after the last heartbeat with the default `Config`.
    assert!(number(failover, "elect_p50_ms") >= 250.0, "got: {failover:?}");
    assert!(
        number(failover, "ready_max_ms") >= number(failover, "elect_max_ms"),
        "got: {failover:?}"
    );

    let degraded = &lines[2];
    assert_eq!("0", degraded["elections"], "the new leaders keep their groups");
    assert!(
        number(degraded, "hb_failed") > 0.0,
        "heartbeats to node 1 fail: {degraded:?}"
    );
}
