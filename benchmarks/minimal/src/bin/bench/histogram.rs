use serde_json::Value;

const LINEAR_BUCKETS: usize = 128;
const SUB_BUCKETS: usize = 64;
const BUCKETS: usize = LINEAR_BUCKETS + 57 * SUB_BUCKETS;

/// A bounded histogram with exact counts and sums and at most 1/64 relative bucket width.
#[derive(Clone, Debug)]
pub(super) struct Histogram {
    buckets: Box<[u64]>,
    pub(super) count: u64,
    total: u128,
    min: u64,
    max: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: Box::default(),
            count: 0,
            total: 0,
            min: u64::MAX,
            max: 0,
        }
    }
}

impl Histogram {
    pub(super) fn record(&mut self, value: u64) {
        if self.buckets.is_empty() {
            self.buckets = vec![0; BUCKETS].into_boxed_slice();
        }
        let bucket = index(value);
        self.buckets[bucket] += 1;
        self.count += 1;
        self.total += u128::from(value);
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }

    pub(super) fn merge(&mut self, other: Self) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = other;
            return;
        }
        for (target, source) in self.buckets.iter_mut().zip(other.buckets.iter()) {
            *target += source;
        }
        self.count += other.count;
        self.total += other.total;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    pub(super) fn percentile(&self, percentile: u64) -> u64 {
        let valid = (1..=100).contains(&percentile);
        assert!(valid, "percentile must be between 1 and 100");
        if self.count == 0 {
            return 0;
        }
        let rank = (u128::from(self.count) * u128::from(percentile)).div_ceil(100);
        let mut seen = 0u128;
        for (bucket, count) in self.buckets.iter().enumerate() {
            seen += u128::from(*count);
            if seen >= rank {
                return upper_bound(bucket).min(self.max);
            }
        }
        unreachable!("histogram counts equal the bucket sum")
    }

    pub(super) fn mean(&self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        (self.total / u128::from(self.count)) as u64
    }

    pub(super) fn latency(&self) -> Value {
        if self.count == 0 {
            return Value::Null;
        }
        serde_json::json!({"samples":self.count,"mean_ns":self.mean(),
            "p50_ns":self.percentile(50),"p95_ns":self.percentile(95),"p99_ns":self.percentile(99)})
    }

    pub(super) fn report(&self) -> Value {
        if self.count == 0 {
            return Value::Null;
        }
        serde_json::json!({"samples":self.count,"min":self.min,"max":self.max,
            "mean":self.mean(),"p50":self.percentile(50),"p90":self.percentile(90),"p99":self.percentile(99)})
    }

    pub(super) fn bytes(&self) -> usize {
        self.buckets.len() * size_of::<u64>()
    }
}

fn index(value: u64) -> usize {
    if value < LINEAR_BUCKETS as u64 {
        return value as usize;
    }
    let exponent = 63 - value.leading_zeros() as usize;
    let shift = exponent - 6;
    let mantissa = (value >> shift) as usize;
    LINEAR_BUCKETS + (exponent - 7) * SUB_BUCKETS + mantissa - SUB_BUCKETS
}

fn upper_bound(bucket: usize) -> u64 {
    if bucket < LINEAR_BUCKETS {
        return bucket as u64;
    }
    let group = (bucket - LINEAR_BUCKETS) / SUB_BUCKETS;
    let mantissa = (bucket - LINEAR_BUCKETS) % SUB_BUCKETS + SUB_BUCKETS;
    let shift = group + 1;
    let lower = (mantissa as u64) << shift;
    lower | ((1u64 << shift) - 1)
}

#[cfg(test)]
mod tests {
    use super::Histogram;
    use super::index;
    use super::upper_bound;

    #[test]
    fn every_bucket_covers_its_value_and_extremes() {
        for value in (0..10000).chain([u64::MAX / 2, u64::MAX]) {
            let bucket = index(value);
            let upper = upper_bound(bucket);
            assert!(upper >= value);
            let width = upper - value;
            assert!(width <= value / 64);
        }
    }

    #[test]
    fn merging_preserves_exact_counts_and_small_percentiles() {
        let mut first = Histogram::default();
        let mut second = Histogram::default();
        for value in 1..=100 {
            if value <= 50 {
                first.record(value);
            } else {
                second.record(value);
            }
        }
        first.merge(second);
        let actual = first.latency();
        let expected = serde_json::json!({"samples":100,"mean_ns":50,"p50_ns":50,"p95_ns":95,"p99_ns":99});
        assert_eq!(expected, actual);
    }
}
