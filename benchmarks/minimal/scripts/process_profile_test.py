import argparse
import unittest

import process_profile as profile


class ProcessProfileTest(unittest.TestCase):
    def test_histogram_returns_bounded_upper_estimates(self):
        for value in list(range(256)) + [1000, 1_000_000, 1_000_000_000, 10**15]:
            histogram = profile.Histogram()
            histogram.record(value)
            estimate = histogram.percentile(99)
            self.assertGreaterEqual(estimate, value)
            self.assertLessEqual(estimate, value + max(1, value // 16))
            self.assertEqual(1, histogram.report()["samples"])

    def test_histories_and_acknowledgements_have_independent_bounds(self):
        args = argparse.Namespace(warmup=0, seconds=1, drain=1, history_capacity=2,
                                  history_every=1, seed=1)
        recorder = profile.Recorder(args)
        for operation in range(4):
            recorder.event({"kind": "write", "operation": operation})
        recorder.acknowledge("key", 5, "new", 1)
        recorder.acknowledge("key", 3, "old", 2)
        _, required = recorder.begin("key", "read")
        self.assertEqual((5, "new", 1), required)
        self.assertEqual([2, 3], [event["operation"] for event in recorder.events])
        self.assertEqual(2, recorder.dropped)

    def test_floor_and_payload_violations_fail_the_oracle(self):
        args = argparse.Namespace(keys=1, hot_key_percent=0, seed=1, payload_bytes=3)
        cluster = object.__new__(profile.Cluster)
        cluster.args = args
        cluster.run_id = "test"
        key = cluster.key(1)
        value = {"version": 5, "value": cluster.value(1)}
        cluster.check_value(key, value, (5, cluster.value(1), 1))
        with self.assertRaises(profile.OracleViolation):
            cluster.check_value(key, value, (6, cluster.value(2), 2))
        corrupt = {"version": 5, "value": "test:1:wrong"}
        with self.assertRaises(profile.OracleViolation):
            cluster.check_value(key, corrupt, None)


if __name__ == "__main__":
    unittest.main()
