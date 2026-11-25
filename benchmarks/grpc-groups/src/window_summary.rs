use std::fmt;

use crate::duration_histogram::DurationHistogram;
use crate::phase::Phase;
use crate::protobuf as pb;

/// What the running nodes measured in one window: counts and rates summed over the nodes, and
/// percentiles over the samples of all of them.
pub(crate) struct WindowSummary {
    phase: Phase,
    cpu_percent: f64,
    memory_kib: u64,
    tasks: u64,
    appends_per_sec: f64,
    rpcs_per_sec: f64,
    votes_per_sec: f64,
    hb_failed: u64,
    hb_rtt_us: DurationHistogram,
    ack_age_us: DurationHistogram,
    quorum_age_max_us: u64,
    elections: u64,
}

impl WindowSummary {
    /// Sum `reports`, one from each running node, and count the elections since `begin_terms`,
    /// which the same nodes reported when the window began.
    pub(crate) fn new(phase: Phase, begin_terms: &[pb::Terms], reports: &[pb::WindowReport]) -> Self {
        let mut summary = WindowSummary {
            phase,
            cpu_percent: 0.0,
            memory_kib: 0,
            tasks: 0,
            appends_per_sec: 0.0,
            rpcs_per_sec: 0.0,
            votes_per_sec: 0.0,
            hb_failed: 0,
            hb_rtt_us: DurationHistogram::default(),
            ack_age_us: DurationHistogram::default(),
            quorum_age_max_us: 0,
            elections: 0,
        };

        for report in reports {
            let per_sec = |count: u64| count as f64 / report.secs;

            summary.cpu_percent += 100.0 * report.cpu_secs / report.secs;
            summary.memory_kib += report.memory_kib;
            summary.tasks += report.tasks;
            summary.appends_per_sec += per_sec(report.appends);
            summary.rpcs_per_sec += per_sec(report.rpcs);
            summary.votes_per_sec += per_sec(report.votes);
            summary.hb_failed += report.hb_failed;
            if let Some(hb_rtt_us) = &report.hb_rtt_us {
                summary.hb_rtt_us.add_pb(hb_rtt_us);
            }
            if let Some(ack_age_us) = &report.ack_age_us {
                summary.ack_age_us.add_pb(ack_age_us);
            }
            summary.quorum_age_max_us = summary.quorum_age_max_us.max(report.quorum_age_max_us);
        }

        let end_terms = reports.iter().filter_map(|report| report.terms.as_ref());
        summary.elections = elections(begin_terms, end_terms);
        summary
    }
}

impl fmt::Display for WindowSummary {
    /// Write one line of tab-separated `name=value` fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let per_group = |value: f64| value / self.phase.groups as f64;

        write!(f, "{}", self.phase)?;
        write!(f, "\tcpu%={:.1}", self.cpu_percent)?;
        write!(
            f,
            "\tmemory_mib={:.0}\tmemory_kib/group={:.0}",
            self.memory_kib as f64 / 1024.0,
            per_group(self.memory_kib as f64)
        )?;
        write!(
            f,
            "\ttasks={}\ttasks/group={:.1}",
            self.tasks,
            per_group(self.tasks as f64)
        )?;
        write!(
            f,
            "\tappends/s={:.0}\trpcs/s={:.0}\tvotes/s={:.0}",
            self.appends_per_sec, self.rpcs_per_sec, self.votes_per_sec
        )?;
        write!(
            f,
            "\thb_rtt_p50_us={}\thb_rtt_p99_us={}\thb_rtt_max_us={}",
            or_na(self.hb_rtt_us.percentile(0.5)),
            or_na(self.hb_rtt_us.percentile(0.99)),
            or_na(self.hb_rtt_us.max())
        )?;
        write!(f, "\thb_failed={}", self.hb_failed)?;
        write!(
            f,
            "\tack_age_p50_ms={}\tack_age_p99_ms={}\tack_age_max_ms={}",
            millis(self.ack_age_us.percentile(0.5)),
            millis(self.ack_age_us.percentile(0.99)),
            millis(self.ack_age_us.max())
        )?;
        write!(f, "\tquorum_age_max_ms={}", millis(Some(self.quorum_age_max_us)))?;
        write!(f, "\telections={}", self.elections)
    }
}

/// How far the highest term of each group grew from `begin` to `end`, the terms that the nodes
/// reported, summed over the groups.
fn elections<'a, B, E>(begin: B, end: E) -> u64
where
    B: IntoIterator<Item = &'a pb::Terms>,
    E: IntoIterator<Item = &'a pb::Terms>,
{
    let begin = highest_terms(begin);
    let end = highest_terms(end);
    end.iter().zip(&begin).map(|(end, begin)| end.saturating_sub(*begin)).sum()
}

/// Return the highest term of each group among the terms that the nodes reported.
pub(crate) fn highest_terms<'a, I>(reports: I) -> Vec<u64>
where I: IntoIterator<Item = &'a pb::Terms> {
    let mut highest = Vec::new();
    for report in reports {
        if highest.len() < report.terms.len() {
            highest.resize(report.terms.len(), 0);
        }
        for (highest, term) in highest.iter_mut().zip(&report.terms) {
            *highest = (*highest).max(*term);
        }
    }
    highest
}

fn or_na<T>(value: Option<T>) -> String
where T: fmt::Display {
    value.map_or_else(|| "n/a".to_string(), |value| value.to_string())
}

/// Format microseconds as milliseconds with one decimal.
fn millis(micros: Option<u64>) -> String {
    micros.map_or_else(|| "n/a".to_string(), |micros| format!("{:.1}", micros as f64 / 1000.0))
}

#[cfg(test)]
mod tests {
    use super::elections;
    use crate::protobuf as pb;

    fn terms(terms: &[u64]) -> pb::Terms {
        pb::Terms { terms: terms.to_vec() }
    }

    #[test]
    fn test_elections_count_how_far_the_highest_term_of_each_group_grew() {
        // Group 0 keeps its leader, group 1 elects once, and group 2 twice; node 2 has not yet
        // seen the second election of group 2.
        let begin = [terms(&[1, 1, 1]), terms(&[1, 1, 1])];
        let end = [terms(&[1, 2, 3]), terms(&[1, 2, 2])];

        assert_eq!(3, elections(&begin, &end));
    }
}
