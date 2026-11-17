import argparse
import http.server
import shutil
import subprocess
import sys
import tempfile
import types
from pathlib import Path
from unittest import mock
import socket
import threading
import time
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


class HttpDeadlineTest(unittest.TestCase):
    def test_progressing_headers_and_body_still_obey_absolute_deadline(self):
        for headers_sent in (False, True):
            with self.subTest(headers_sent=headers_sent):
                listener = socket.socket()
                listener.bind(("127.0.0.1", 0))
                listener.listen()
                address = "%s:%s" % listener.getsockname()

                def serve():
                    with listener:
                        connection, _ = listener.accept()
                        with connection:
                            connection.recv(65536)
                            header = b"HTTP/1.1 200 OK\r\nContent-Length: 40\r\n\r\n"
                            if headers_sent:
                                connection.sendall(header)
                            data = b"x" * 40 if headers_sent else header
                            for byte in data:
                                try:
                                    connection.sendall(bytes([byte]))
                                except (BrokenPipeError, ConnectionResetError):
                                    return
                                time.sleep(0.015)

                thread = threading.Thread(target=serve, daemon=True)
                thread.start()
                client = profile.Http()
                started = time.monotonic()
                with self.assertRaises(TimeoutError):
                    client.request(address, "/slow", None, started + 0.05)
                elapsed = time.monotonic() - started
                self.assertLess(elapsed, 0.2)
                client.close_all()
                thread.join(1)
                self.assertFalse(thread.is_alive())


class LifecycleTest(unittest.TestCase):
    def test_recovery_waits_for_an_acknowledgement_after_leader_discovery(self):
        cluster = object.__new__(profile.Cluster)
        cluster.args = argparse.Namespace(request_timeout=0.1)
        cluster.recorder = types.SimpleNamespace(deadline=time.monotonic()+1, origin=time.monotonic_ns())
        cluster.run_id = "test"
        cluster.shutdown = threading.Event()
        cluster.leading = mock.Mock(return_value=2)
        cluster.api = mock.Mock(side_effect=[profile.AvailabilityError("lease warming"), {"log_id": {"index": 11}}])
        event = {"target": 1, "started_ns": 0}
        cluster.recovery_probe(event)
        self.assertEqual(2, cluster.api.call_count)
        self.assertEqual(2, event["replacement_leader"])
        self.assertEqual({"index": 11}, event["recovery_probe_log_id"])
        self.assertEqual(["lease warming"], event["recovery_probe_errors"])
        self.assertGreaterEqual(event["outage_upper_bound_ns"], 0)

    def test_named_endpoints_fail_before_any_dns_work(self):
        client = profile.Http()
        with mock.patch.object(socket, "getaddrinfo", side_effect=AssertionError("blocking DNS")) as resolver:
            with self.assertRaises(ValueError):
                client.request("localhost:80", "/", None, time.monotonic() + 1)
        resolver.assert_not_called()

    def test_reused_connection_is_tracked_after_each_close(self):
        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_GET(self):
                self.send_response(200)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"{}")

            def log_message(self, *args):
                return

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        address = "%s:%s" % server.server_address
        client = profile.Http()
        first = client.request(address, "/", None, time.monotonic() + 1)
        self.assertEqual({}, first)
        original = next(iter(client.connections))
        client.close_all()
        second = client.request(address, "/", None, time.monotonic() + 1)
        self.assertEqual({}, second)
        self.assertEqual({original}, client.connections)
        client.close_all()
        self.assertIsNone(original.sock)
        server.shutdown()
        server.server_close()
        thread.join(1)
        self.assertFalse(thread.is_alive())

    def test_incomplete_generator_fails_the_run(self):
        release = threading.Event()
        original = profile.Recorder

        class GatedRecorder(original):
            def outcome(self, *args):
                release.wait(1)
                return super().outcome(*args)

        args = argparse.Namespace(warmup=0, seconds=0.005, drain=0.005, history_capacity=2,
            history_every=1, seed=1, write_rate=1_000_000, read_rate=0, clients=1,
            read_clients=0, queue_capacity=1, request_timeout=0.05, scenario="normal")
        cluster = types.SimpleNamespace(args=args, http=types.SimpleNamespace(close_all=lambda: None), shutdown=threading.Event())
        cluster.operation = lambda kind, operation, scheduled: cluster.recorder.outcome(
            kind, scheduled, time.monotonic(), "completed")
        timer = threading.Timer(0.05, release.set)
        timer.start()
        with mock.patch.object(profile, "Recorder", GatedRecorder):
            with self.assertRaisesRegex(TimeoutError, "generator"):
                profile.workload(cluster)
        timer.join(1)

    def test_metadata_retries_a_stale_pooled_connection(self):
        args = argparse.Namespace(attach=None, port_base=41000)
        cluster = profile.Cluster(args, Path("unused"))
        cluster.nodes = {1: {}}
        cluster.api = mock.Mock(side_effect=[http.client.RemoteDisconnected("old process"), {"count": 2}])
        reports = cluster.read_reports("/benchmark/stats")
        self.assertEqual({1: {"count": 2}}, reports)
        self.assertEqual(2, cluster.api.call_count)
        self.assertEqual(1, cluster.control_error_count)

    def test_healing_attempts_every_peer_even_after_one_endpoint_fails(self):
        cluster = object.__new__(profile.Cluster)
        cluster.nodes = {node: {"control_addr": str(node)} for node in (1, 2, 3)}
        request = mock.Mock(side_effect=[ConnectionRefusedError("down"), [], []])
        cluster.http = types.SimpleNamespace(request=request)
        cluster.shutdown = threading.Event()
        cluster.shutdown.set()
        with self.assertRaisesRegex(profile.AvailabilityError, "node 1"):
            cluster.partition(None)
        self.assertEqual(["1", "2", "3"], [call.args[0] for call in request.call_args_list])
        self.assertEqual([[], [], []], [call.args[2] for call in request.call_args_list])

    def test_shutdown_prevents_a_gated_controller_from_spawning_after_cleanup(self):
        directory = tempfile.mkdtemp(prefix="openraft-process-lifecycle-")
        args = argparse.Namespace(attach=None, port_base=41000, backend="wal", binary=Path(sys.executable),
            snapshot_logs=None, request_timeout=0.05, warmup=0, seconds=0.1, drain=0.1,
            history_capacity=2, history_every=1, seed=1, scenario="restart")
        cluster = profile.Cluster(args, Path(directory))
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(5)"])
        cluster.processes[1] = child
        runner = profile.Runner(cluster)
        cluster.fault = lambda: cluster.start(2)
        entered = threading.Event()
        release = threading.Event()
        spawn = cluster.spawn
        errors = []

        def gated_spawn(*args):
            entered.set()
            release.wait(1)
            spawn(*args)

        def close():
            try:
                cluster.close()
            except Exception as error:
                errors.append(error)

        cluster.spawn = gated_spawn
        runner.control = threading.Thread(target=runner.fault)
        with mock.patch.object(profile.subprocess, "Popen", side_effect=AssertionError("spawn after shutdown")) as creating:
            runner.control.start()
            self.assertTrue(entered.wait(1))
            closing = threading.Thread(target=close)
            closing.start()
            self.assertTrue(cluster.shutdown.wait(1))
            release.set()
            closing.join(1)
            runner.control.join(1)
            creating.assert_not_called()
        self.assertFalse(closing.is_alive())
        self.assertFalse(runner.control.is_alive())
        self.assertEqual([], errors)
        self.assertIsNotNone(child.poll())
        shutil.rmtree(directory)

    def test_report_write_failure_still_stops_owned_processes(self):
        children = []

        class FailingCluster(profile.Cluster):
            def initialize(self):
                child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
                self.processes[1] = child
                children.append(child)
                raise RuntimeError("injected startup failure")

        directory = tempfile.mkdtemp(prefix="openraft-process-profile-")
        args = argparse.Namespace(output=Path(directory) / "run", backend="wal", attach=None, port_base=41000)
        with mock.patch.object(profile, "parse_args", return_value=args):
            with mock.patch.object(profile, "Cluster", FailingCluster):
                with mock.patch.object(Path, "write_text", side_effect=OSError("injected full disk")):
                    with self.assertRaisesRegex(OSError, "full disk"):
                        profile.main()
        self.assertEqual(1, len(children))
        self.assertIsNotNone(children[0].poll())
        shutil.rmtree(directory)


if __name__ == "__main__":
    unittest.main()
