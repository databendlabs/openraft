use std::fmt;

use crate::failover_summary::millis;
use crate::failover_summary::percentile;
use crate::phase::Phase;
use crate::protobuf as pb;
use crate::window_summary::highest_terms;

/// How the node that returned from a fault rejoined the groups, and how many groups held an
/// election meanwhile.
pub(crate) struct RejoinSummary {
    phase: Phase,

    /// The time from the return until the node stopped leading each group that it led and
    /// stopped leading, in nanoseconds, sorted.
    stepdown_ns: Vec<u64>,

    /// The time from the return until the node rejoined each group that it rejoined, in
    /// nanoseconds, sorted.
    rejoin_ns: Vec<u64>,

    /// The groups that the node had not rejoined when the phase ended.
    unrejoined: usize,

    /// The groups whose highest term grew during the phase.
    disrupted: usize,
}

impl RejoinSummary {
    /// Summarize `rejoins`, which the returning node recorded after `returned_ns`, the time of its
    /// return. `begin_terms` are the terms that the other nodes reported before the return, and
    /// `end_terms` those that every node reported when the phase ended.
    pub(crate) fn new(
        phase: Phase,
        returned_ns: u64,
        rejoins: &pb::Rejoins,
        begin_terms: &[pb::Terms],
        end_terms: &[pb::Terms],
    ) -> Self {
        let mut stepdown_ns = Vec::new();
        let mut rejoin_ns = Vec::new();
        let mut unrejoined = 0;
        for rejoin in &rejoins.rejoins {
            if rejoin.led && rejoin.stepdown_ns > 0 {
                stepdown_ns.push(rejoin.stepdown_ns.saturating_sub(returned_ns));
            }
            if rejoin.rejoined_ns > 0 {
                rejoin_ns.push(rejoin.rejoined_ns.saturating_sub(returned_ns));
            } else {
                unrejoined += 1;
            }
        }
        stepdown_ns.sort_unstable();
        rejoin_ns.sort_unstable();

        let begin = highest_terms(begin_terms);
        let end = highest_terms(end_terms);
        let disrupted = end.iter().zip(&begin).filter(|(end, begin)| end > begin).count();

        RejoinSummary {
            phase,
            stepdown_ns,
            rejoin_ns,
            unrejoined,
            disrupted,
        }
    }
}

impl fmt::Display for RejoinSummary {
    /// Write one line of tab-separated `name=value` fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.phase)?;
        for (name, sorted) in [("stepdown", &self.stepdown_ns), ("rejoin", &self.rejoin_ns)] {
            write!(
                f,
                "\t{name}_p99_ms={}\t{name}_max_ms={}",
                millis(percentile(sorted, 0.99)),
                millis(sorted.last().copied())
            )?;
        }
        write!(f, "\tunrejoined={}\tdisrupted={}", self.unrejoined, self.disrupted)
    }
}

#[cfg(test)]
mod tests {
    use super::RejoinSummary;
    use crate::phase::Phase;
    use crate::protobuf as pb;
    use crate::scenario::Scenario;

    const MS: u64 = 1_000_000;

    fn rejoin(group_id: u64, led: bool, stepdown_ms: u64, rejoined_ms: u64) -> pb::Rejoin {
        pb::Rejoin {
            group_id,
            led,
            stepdown_ns: stepdown_ms * MS,
            rejoined_ns: rejoined_ms * MS,
        }
    }

    fn terms(terms: &[u64]) -> pb::Terms {
        pb::Terms { terms: terms.to_vec() }
    }

    #[test]
    fn test_a_summary_times_the_rejoin_from_the_return() {
        let phase = Phase {
            scenario: Scenario::Hang,
            name: "rejoin",
            nodes: 3,
            groups: 3,
        };
        // The node returns at 1,000 ms. It led group 0, stepped down and rejoined it; it rejoined
        // group 1 at once, and had not rejoined group 2 by the end. Only group 1 elected a leader
        // meanwhile.
        let rejoins = pb::Rejoins {
            rejoins: vec![
                rejoin(0, true, 1030, 1040),
                rejoin(1, false, 0, 1010),
                rejoin(2, false, 0, 0),
            ],
        };
        let begin_terms = [terms(&[3, 2, 2]), terms(&[3, 2, 2])];
        let end_terms = [terms(&[3, 3, 2]), terms(&[3, 3, 2]), terms(&[3, 3, 2])];

        let summary = RejoinSummary::new(phase, 1000 * MS, &rejoins, &begin_terms, &end_terms);

        assert_eq!(vec![30 * MS], summary.stepdown_ns);
        assert_eq!(vec![10 * MS, 40 * MS], summary.rejoin_ns);
        assert_eq!(1, summary.unrejoined);
        assert_eq!(1, summary.disrupted);

        let line = summary.to_string();
        assert!(
            line.contains("\tstepdown_p99_ms=30.0\tstepdown_max_ms=30.0\t"),
            "got: {line}"
        );
        assert!(
            line.ends_with("\trejoin_p99_ms=40.0\trejoin_max_ms=40.0\tunrejoined=1\tdisrupted=1"),
            "got: {line}"
        );
    }
}
