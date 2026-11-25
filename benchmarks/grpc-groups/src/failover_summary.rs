use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use crate::phase::Phase;
use crate::protobuf as pb;

/// How fast the surviving nodes took over the groups that the failed node led.
pub(crate) struct FailoverSummary {
    phase: Phase,

    /// How many groups the failed node led.
    affected: usize,

    /// The time from the fault until a surviving member became leader, of each group that elected
    /// one, in nanoseconds, sorted.
    elect_ns: Vec<u64>,

    /// The time from the fault until a new leader committed an entry of its term, of each group
    /// that has one, in nanoseconds, sorted.
    ready_ns: Vec<u64>,

    /// The groups that needed more than one term to elect their new leader.
    retried: usize,

    /// The groups without a new leader that committed an entry of its term.
    unrecovered: usize,

    /// The other groups, which the failed node did not lead, whose leader changed.
    moved: usize,
}

impl FailoverSummary {
    /// Summarize `takeovers`, which the surviving nodes recorded of the `affected` groups after
    /// `faulted_ns`, the time of the fault. `moved` counts the other groups whose leader changed.
    pub(crate) fn new(
        phase: Phase,
        affected: &[u64],
        faulted_ns: u64,
        takeovers: &[pb::Takeovers],
        moved: usize,
    ) -> Self {
        // A group may change its leader twice: the first takeover elects, the first commit is ready.
        let mut first_elected = BTreeMap::<u64, &pb::Takeover>::new();
        let mut first_ready = BTreeMap::<u64, u64>::new();
        for takeover in takeovers.iter().flat_map(|takeovers| &takeovers.takeovers) {
            let elected = first_elected.entry(takeover.group_id).or_insert(takeover);
            if takeover.elected_ns < elected.elected_ns {
                *elected = takeover;
            }
            if takeover.ready_ns > 0 {
                let ready = first_ready.entry(takeover.group_id).or_insert(takeover.ready_ns);
                *ready = (*ready).min(takeover.ready_ns);
            }
        }

        let mut summary = FailoverSummary {
            phase,
            affected: affected.len(),
            elect_ns: Vec::new(),
            ready_ns: Vec::new(),
            retried: 0,
            unrecovered: 0,
            moved,
        };
        for group_id in affected {
            if let Some(elected) = first_elected.get(group_id) {
                summary.elect_ns.push(elected.elected_ns.saturating_sub(faulted_ns));
                if elected.terms > 1 {
                    summary.retried += 1;
                }
            }
            match first_ready.get(group_id) {
                Some(ready_ns) => summary.ready_ns.push(ready_ns.saturating_sub(faulted_ns)),
                None => summary.unrecovered += 1,
            }
        }
        summary.elect_ns.sort_unstable();
        summary.ready_ns.sort_unstable();
        summary
    }
}

/// Return the groups that a new leader has taken over and committed an entry of its term in.
pub(crate) fn ready_groups(takeovers: &[pb::Takeovers]) -> BTreeSet<u64> {
    let takeovers = takeovers.iter().flat_map(|takeovers| &takeovers.takeovers);
    takeovers.filter(|takeover| takeover.ready_ns > 0).map(|takeover| takeover.group_id).collect()
}

impl fmt::Display for FailoverSummary {
    /// Write one line of tab-separated `name=value` fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\taffected={}", self.phase, self.affected)?;
        for (name, sorted) in [("elect", &self.elect_ns), ("ready", &self.ready_ns)] {
            write!(
                f,
                "\t{name}_p50_ms={}\t{name}_p90_ms={}\t{name}_p99_ms={}\t{name}_max_ms={}",
                millis(percentile(sorted, 0.5)),
                millis(percentile(sorted, 0.9)),
                millis(percentile(sorted, 0.99)),
                millis(sorted.last().copied())
            )?;
        }
        write!(
            f,
            "\tretried={}\tunrecovered={}\tmoved={}",
            self.retried, self.unrecovered, self.moved
        )
    }
}

/// Return the `p` percentile of `sorted`, `p` from 0 to 1, by the nearest rank.
pub(crate) fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// Format nanoseconds as milliseconds with one decimal.
pub(crate) fn millis(nanos: Option<u64>) -> String {
    nanos.map_or_else(|| "n/a".to_string(), |nanos| format!("{:.1}", nanos as f64 / 1e6))
}

#[cfg(test)]
mod tests {
    use super::FailoverSummary;
    use super::percentile;
    use super::ready_groups;
    use crate::phase::Phase;
    use crate::protobuf as pb;
    use crate::scenario::Scenario;

    const MS: u64 = 1_000_000;

    fn takeover(group_id: u64, elected_ms: u64, ready_ms: u64, terms: u64) -> pb::Takeover {
        pb::Takeover {
            group_id,
            elected_ns: elected_ms * MS,
            ready_ns: ready_ms * MS,
            terms,
        }
    }

    #[test]
    fn test_percentile_by_nearest_rank() {
        let sorted = (1..=100).collect::<Vec<u64>>();

        assert_eq!(None, percentile(&[], 0.5));
        assert_eq!(Some(50), percentile(&sorted, 0.5));
        assert_eq!(Some(99), percentile(&sorted, 0.99));
        assert_eq!(Some(1), percentile(&sorted, 0.0));
    }

    #[test]
    fn test_a_summary_takes_the_first_takeover_of_each_group() {
        let phase = Phase {
            scenario: Scenario::Crash,
            name: "failover",
            nodes: 3,
            groups: 9,
        };
        // The fault is at 1,000 ms. Node 2 takes over group 0; nodes 2 and 3 both lead group 3, in
        // turn; group 6 elects node 3 in its second term, which commits nothing yet.
        let node_2 = pb::Takeovers {
            takeovers: vec![takeover(0, 1500, 1501, 1), takeover(3, 1450, 1460, 1)],
        };
        let node_3 = pb::Takeovers {
            takeovers: vec![takeover(3, 1700, 1710, 2), takeover(6, 1600, 0, 2)],
        };
        let takeovers = [node_2, node_3];

        let summary = FailoverSummary::new(phase, &[0, 3, 6], 1000 * MS, &takeovers, 0);

        assert_eq!(vec![450 * MS, 500 * MS, 600 * MS], summary.elect_ns);
        assert_eq!(vec![460 * MS, 501 * MS], summary.ready_ns);
        assert_eq!(1, summary.retried);
        assert_eq!(1, summary.unrecovered);
        assert_eq!(
            [0, 3].into_iter().collect::<std::collections::BTreeSet<u64>>(),
            ready_groups(&takeovers)
        );

        let line = summary.to_string();
        assert!(line.contains("\taffected=3\telect_p50_ms=500.0\t"), "got: {line}");
        assert!(
            line.contains("\tready_max_ms=501.0\tretried=1\tunrecovered=1\tmoved=0"),
            "got: {line}"
        );
    }
}
