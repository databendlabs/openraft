use base2histogram::Histogram;

use crate::protobuf as pb;

/// The width of the log scale. Its percentiles are within 1.6% of the true value, as in the write
/// burst of idle-groups.
const WIDTH: usize = 6;

/// A log-scale histogram of durations, in the unit its caller chooses, with the exact largest
/// value.
///
/// A node sends its histogram to the driver as [`pb::Histogram`], and the driver adds the
/// histograms of all nodes, so that percentiles cover the samples of all of them.
pub(crate) struct DurationHistogram {
    histogram: Histogram,
    max: u64,
}

impl Default for DurationHistogram {
    fn default() -> Self {
        Self {
            histogram: Histogram::with_log_scale(WIDTH, 1),
            max: 0,
        }
    }
}

impl DurationHistogram {
    pub(crate) fn record(&mut self, value: u64) {
        self.histogram.record(value);
        self.max = self.max.max(value);
    }

    /// Return the `p` percentile, `p` from 0 to 1, or `None` without any sample.
    pub(crate) fn percentile(&self, p: f64) -> Option<u64> {
        if self.histogram.total() == 0 {
            return None;
        }
        Some(self.histogram.percentile(p))
    }

    /// Return the largest sample, or `None` without any sample.
    pub(crate) fn max(&self) -> Option<u64> {
        if self.histogram.total() == 0 {
            return None;
        }
        Some(self.max)
    }

    pub(crate) fn to_pb(&self) -> pb::Histogram {
        let buckets = (0..self.histogram.num_buckets())
            .map(|index| self.histogram.bucket(index))
            .filter(|bucket| bucket.count() > 0)
            .map(|bucket| pb::Bucket {
                value: bucket.left(),
                count: bucket.count(),
            })
            .collect();

        pb::Histogram { buckets, max: self.max }
    }

    /// Add the samples of `histogram`, which another process recorded at the same width.
    pub(crate) fn add_pb(&mut self, histogram: &pb::Histogram) {
        for bucket in &histogram.buckets {
            self.histogram.record_n(bucket.value, bucket.count);
        }
        self.max = self.max.max(histogram.max);
    }
}

#[cfg(test)]
mod tests {
    use super::DurationHistogram;

    #[test]
    fn test_an_empty_histogram_has_no_percentile() {
        let histogram = DurationHistogram::default();

        assert_eq!(None, histogram.percentile(0.5));
        assert_eq!(None, histogram.max());
    }

    #[test]
    fn test_added_histograms_cover_the_samples_of_both() {
        let mut first = DurationHistogram::default();
        let mut second = DurationHistogram::default();
        for value in 1..=100 {
            first.record(value);
            second.record(value + 1000);
        }

        let mut both = DurationHistogram::default();
        both.add_pb(&first.to_pb());
        both.add_pb(&second.to_pb());

        // The true median is 100 and the true p99 is 1098: a percentile is within 1.6% of it.
        let median = both.percentile(0.5).unwrap();
        assert!((98..=102).contains(&median), "median: {median}");
        let p99 = both.percentile(0.99).unwrap();
        assert!((1080..=1116).contains(&p99), "p99: {p99}");
        assert_eq!(Some(1100), both.max());
    }
}
