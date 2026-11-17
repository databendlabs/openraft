#!/usr/bin/env python3
"""Run a bounded, paced HTTP workload against WAL or RocksDB example processes.

Spawn three local nodes, or --attach to a JSON manifest containing `nodes` with
id/api_addr/raft_addr (and optional control_addr). All artifacts stay in --output.
Existing HTTP read endpoints use ReadIndex; replication is the unary adapter.
"""

import argparse
import collections
import errno
import hashlib
import http.client
import json
import math
import os
import platform
import queue
import socket
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[3]
BACKENDS = {
    "wal": ("raft-kv-log-wal-sm-mem", "raft-kv-log-wal-sm-mem"),
    "rocks": ("raft-kv-rocksdb", "raft-key-value-rocks"),
}


class AvailabilityError(RuntimeError):
    pass


class OracleViolation(RuntimeError):
    pass


class Histogram:
    def __init__(self):
        self.buckets = [0] * 1025
        self.count = 0
        self.total = 0

    def record(self, value):
        value = max(0, int(value))
        exponent = max(0, value.bit_length() - 5)
        index = min(1024, exponent * 16 + (value >> exponent))
        self.buckets[index] += 1
        self.count += 1
        self.total += value

    def percentile(self, percent):
        rank = math.ceil(self.count * percent / 100)
        seen = 0
        for index, count in enumerate(self.buckets):
            seen += count
            if count and seen >= rank:
                exponent = max(0, (index - 16) // 16)
                mantissa = index - exponent * 16
                return ((mantissa + 1) << exponent) - 1
        return None

    def report(self):
        return {"samples": self.count, "mean_ns": self.total // self.count if self.count else None,
                "p50_ns": self.percentile(50), "p99_ns": self.percentile(99),
                "precision": "16 sub-buckets per power of two"}


class Http:
    def __init__(self):
        self.local = threading.local()
        self.lock = threading.Lock()
        self.connections = set()
        self.sent = 0
        self.received = 0

    def request(self, address, path, body, deadline):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("logical request deadline reached")
        parsed = urlsplit(address if "://" in address else "http://" + address)
        if parsed.scheme != "http" or not parsed.hostname:
            raise ValueError("node addresses must be HTTP endpoints")
        endpoint = (parsed.hostname, parsed.port or 80)
        connections = getattr(self.local, "connections", None)
        if connections is None:
            connections = self.local.connections = {}
        connection = connections.get(endpoint)
        if connection is None:
            connection = http.client.HTTPConnection(*endpoint, timeout=remaining)
            connections[endpoint] = connection
            with self.lock:
                self.connections.add(connection)
        connection.timeout = remaining
        if connection.sock:
            connection.sock.settimeout(remaining)
        encoded = None if body is None else json.dumps(body, separators=(",", ":")).encode()
        try:
            connection.request("GET" if body is None else "POST", path, encoded,
                               {"Content-Type": "application/json"})
            response = connection.getresponse()
            data = response.read()
        except (OSError, http.client.HTTPException):
            connection.close()
            del connections[endpoint]
            with self.lock:
                self.connections.discard(connection)
            raise
        with self.lock:
            self.sent += len(encoded or b"")
            self.received += len(data)
        if response.status != 200:
            raise AvailabilityError(f"HTTP {response.status}: {data[:300]!r}")
        return json.loads(data)

    def close_all(self):
        with self.lock:
            connections = list(self.connections)
            self.connections.clear()
        for connection in connections:
            sock = connection.sock
            if sock:
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError as error:
                    if error.errno not in (errno.ENOTCONN, errno.EBADF):
                        raise
            connection.close()


def unwrap(reply):
    if not isinstance(reply, dict) or "Ok" not in reply:
        raise AvailabilityError(f"application error: {reply}")
    return reply["Ok"]


def mixed(value):
    value = (value ^ (value >> 30)) * 0xBF58476D1CE4E5B9 & ((1 << 64) - 1)
    value = (value ^ (value >> 27)) * 0x94D049BB133111EB & ((1 << 64) - 1)
    return value ^ (value >> 31)


class Recorder:
    def __init__(self, args):
        self.args = args
        self.lock = threading.Lock()
        self.origin = time.monotonic_ns()
        self.epoch = time.monotonic()
        self.start = self.epoch + args.warmup
        self.end = self.start + args.seconds
        self.deadline = self.end + args.drain
        self.events = collections.deque(maxlen=args.history_capacity)
        self.dropped = 0
        self.counts = {kind: collections.Counter() for kind in ("write", "read")}
        self.histograms = {kind: {phase: Histogram() for phase in ("latency", "client_delay", "service")}
                           for kind in self.counts}
        self.acknowledged = {}
        self.oracle_errors = collections.deque(maxlen=8)
        self.oracle_error_count = 0
        self.peak_queue = collections.Counter()
        self.attempts = {kind: collections.Counter() for kind in self.counts}
        self.last_success = None

    def event(self, event, always=False):
        operation = event.get("operation", 0)
        stream = 0x72656164 if event.get("kind") == "read" else 0x7772697465
        sampled = self.args.history_every and mixed(operation ^ stream ^ self.args.seed) % self.args.history_every == 0
        if not always and not sampled:
            return
        with self.lock:
            if len(self.events) == self.events.maxlen:
                self.dropped += 1
            self.events.append(event)

    def outcome(self, kind, scheduled, invoked, outcome):
        finished = time.monotonic()
        with self.lock:
            if self.start <= scheduled < self.end:
                self.counts[kind][outcome] += 1
                if outcome == "completed":
                    self.histograms[kind]["latency"].record((finished - scheduled) * 1e9)
                if invoked is not None:
                    self.histograms[kind]["client_delay"].record((invoked - scheduled) * 1e9)
                    self.histograms[kind]["service"].record((finished - invoked) * 1e9)
            if outcome == "completed":
                self.last_success = finished
                if self.start <= finished < self.end:
                    self.counts[kind]["completed_in_window"] += 1

    def acknowledge(self, key, version, value, operation):
        with self.lock:
            old = self.acknowledged.get(key)
            if old is None or version > old[0]:
                self.acknowledged[key] = (version, value, operation)

    def begin(self, key, kind):
        with self.lock:
            return time.monotonic(), self.acknowledged.get(key) if kind == "read" else None

    def report(self):
        with self.lock:
            return {kind: {"outcomes": dict(self.counts[kind]), "attempts": dict(self.attempts[kind]),
                           "peak_waiting_requests": self.peak_queue[kind],
                           "operations_per_second": self.counts[kind]["completed_in_window"] / self.args.seconds,
                           **{name: histogram.report() for name, histogram in self.histograms[kind].items()}}
                    for kind in self.counts}


class Cluster:
    def __init__(self, args, output):
        self.args = args
        self.output = output
        self.http = Http()
        self.processes = {}
        self.commands = {}
        self.logs = []
        self.leader_id = 1
        self.run_id = uuid.uuid4().hex
        self.nodes = self.attach() if args.attach else self.local_nodes()
        self.events = []
        self.recorder = None
        self.shutdown = threading.Event()

    def attach(self):
        manifest = json.loads(self.args.attach.read_text())
        nodes = manifest["nodes"]
        if len(nodes) < 3 or len({node["id"] for node in nodes}) != len(nodes):
            raise ValueError("attach manifest needs at least three distinct node IDs")
        for node in nodes:
            if not isinstance(node["id"], int) or node["id"] <= 0 or not node.get("api_addr") or not node.get("raft_addr"):
                raise ValueError("each attached node requires a positive ID, api_addr and raft_addr")
            if not node.get("control_addr"):
                raise ValueError("attached benchmark nodes require the explicit control listener")
        return {node["id"]: node for node in nodes}

    def local_nodes(self):
        return {node: {"id": node, "api_addr": f"127.0.0.1:{self.args.port_base + node}",
                       "raft_addr": f"127.0.0.1:{self.args.port_base + 1000 + node}",
                       "control_addr": f"127.0.0.1:{self.args.port_base + 2000 + node}"}
                for node in (1, 2, 3)}

    def start(self, node):
        if self.args.attach:
            raise ValueError("attach mode does not own node processes")
        crate, binary = BACKENDS[self.args.backend]
        executable = self.args.binary or ROOT / "examples" / crate / "target/release" / binary
        directory = self.output / f"node-{node}"
        directory.mkdir(exist_ok=True)
        options = self.nodes[node]
        command = [str(executable), "--id", str(node), "--api-addr", options["api_addr"],
                   "--raft-addr", options["raft_addr"], "--data-dir", str(directory / "data"),
                   "--benchmark-control-addr", options["control_addr"]]
        if self.args.backend == "rocks":
            command += ["--snapshot-logs", str(self.args.snapshot_logs), "--snapshot-keep", "0"]
        log = (directory / "process.log").open("ab")
        self.logs.append(log)
        self.processes[node] = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        self.commands[node] = command
        self.wait(lambda: self.api(node, "/metrics"), f"node {node} API readiness")
        self.wait(lambda: self.http.request(options["control_addr"], "/status", None, time.monotonic() + 1),
                  f"node {node} control readiness")

    def stop(self, node):
        process = self.processes[node]
        if process.poll() is None:
            process.kill()
        process.wait(timeout=10)

    def api(self, node, path, body=None, deadline=None):
        deadline = deadline or time.monotonic() + self.args.request_timeout
        result = self.http.request(self.nodes[node]["api_addr"], path, body, deadline)
        return unwrap(result)

    def wait(self, check, label, seconds=30):
        deadline = time.monotonic() + seconds
        last = None
        while time.monotonic() < deadline:
            try:
                result = check()
                if result is not None and result is not False:
                    return result
            except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as error:
                last = str(error)
            time.sleep(0.02)
        raise TimeoutError(f"{label}: {last}")

    def leading(self, exclude=None):
        for node in self.nodes:
            if node == exclude:
                continue
            try:
                metrics = self.api(node, "/metrics", deadline=time.monotonic() + 0.3)
            except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError):
                continue
            leader = metrics.get("current_leader")
            if leader in self.nodes and leader != exclude:
                probe = self.api(leader, "/metrics", deadline=time.monotonic() + 0.3)
                if probe.get("state") == "Leader":
                    self.leader_id = leader
                    return leader
        return None

    def initialize(self):
        for node in self.nodes:
            self.start(node)
        self.api(1, "/init", [])
        self.wait(lambda: self.leading(), "initial leadership")
        for node in (2, 3):
            self.api(1, "/add-learner", {"node_id": node, "api_addr": self.nodes[node]["api_addr"],
                                                "raft_addr": self.nodes[node]["raft_addr"]})
        self.api(1, "/change-membership", [1, 2, 3])
        self.wait(lambda: self.leading(), "three-voter leadership")

    def key(self, operation):
        value = mixed(operation ^ self.args.seed)
        key = 0 if value % 100 < self.args.hot_key_percent else value % self.args.keys
        return f"bench/{self.run_id}/{key}"

    def value(self, operation):
        return f"{self.run_id}:{operation}:" + "x" * self.args.payload_bytes

    def operation(self, kind, operation, scheduled):
        key = self.key(operation)
        invoked, required = self.recorder.begin(key, kind)
        started_ns = int((invoked - self.recorder.epoch) * 1e9)
        deadline = min(scheduled + self.args.request_timeout, self.recorder.deadline)
        retries = 0
        outcome = "timed_out"
        while time.monotonic() < deadline:
            target = self.args.read_on_follower if kind == "read" and self.args.read_on_follower else self.leader_id
            path = "/write" if kind == "write" else ("/follower_read" if self.args.read_on_follower else "/linearizable_read")
            body = {"Set": {"key": key, "value": self.value(operation)}} if kind == "write" else key
            attempt = {"kind": kind, "operation": operation, "key": key, "target": target,
                       "invoked_ns": started_ns, "attempt": retries, "scheduled_ns": int((scheduled - self.recorder.epoch) * 1e9)}
            try:
                attempt["attempt_invoked_ns"] = time.monotonic_ns() - self.recorder.origin
                with self.recorder.lock:
                    self.recorder.attempts[kind]["submitted"] += 1
                    self.recorder.attempts[kind]["retry_attempts"] += int(retries > 0)
                response = self.api(target, path, body, deadline)
                if kind == "write":
                    value = response["data"]["value"]
                    if value is None or value["value"] != self.value(operation):
                        raise OracleViolation("write response does not match its logical operation")
                    self.recorder.acknowledge(key, value["version"], value["value"], operation)
                    attempt.update(log_id=response["log_id"], value=value)
                else:
                    value = response["value"]
                    self.check_value(key, value, required)
                    attempt.update(value=value, required_version=required[0] if required else 0)
                outcome = "completed"
                attempt.update(outcome=outcome, completed_ns=time.monotonic_ns() - self.recorder.origin)
                self.recorder.event(attempt)
                break
            except OracleViolation as error:
                outcome = "oracle_violation"
                with self.recorder.lock:
                    self.recorder.oracle_errors.append(str(error))
                    self.recorder.oracle_error_count += 1
                attempt.update(outcome=outcome, error=str(error))
                self.recorder.event(attempt, always=True)
                break
            except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as error:
                attempt.update(outcome="uncertain" if kind == "write" else "availability_error", error=str(error),
                               completed_ns=time.monotonic_ns() - self.recorder.origin)
                self.recorder.event(attempt)
                with self.recorder.lock:
                    self.recorder.attempts[kind]["availability_errors"] += 1
                    self.recorder.attempts[kind]["uncertain_attempts"] += int(kind == "write")
                retries += 1
                try:
                    self.leading()
                except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as discovery:
                    attempt["leader_discovery_error"] = str(discovery)
                time.sleep(min(0.01, max(0, deadline - time.monotonic())))
        if outcome != "completed" and time.monotonic() >= self.recorder.deadline:
            outcome = "pending_at_drain_deadline"
        self.recorder.outcome(kind, scheduled, invoked, outcome)

    def check_value(self, key, value, required):
        version = value["version"] if value else 0
        if required and version < required[0]:
            raise OracleViolation(f"{key}: read {version} behind acknowledged {required[0]}")
        if required and version == required[0] and value["value"] != required[1]:
            raise OracleViolation(f"{key}: acknowledged version has another value")
        if value:
            parts = value["value"].split(":", 2)
            if len(parts) != 3:
                raise OracleViolation(f"{key}: malformed application value")
            prefix, operation, payload = parts
            if prefix != self.run_id or not operation.isdecimal() or payload != "x" * self.args.payload_bytes:
                raise OracleViolation(f"{key}: corrupted application payload")
            if self.key(int(operation)) != key:
                raise OracleViolation(f"{key}: value belongs to a different key")

    def partition(self, isolated):
        for node, options in self.nodes.items():
            targets = ["*"] if node == isolated else [str(isolated)] if isolated else []
            self.http.request(options["control_addr"], "/partition", targets, time.monotonic() + 2)

    def fault(self):
        recorder = self.recorder
        wait = recorder.start + self.args.fault_after - time.monotonic()
        if wait > 0:
            if self.shutdown.wait(wait):
                return
        recovery_deadline = self.recorder.deadline + 30
        scenario = self.args.scenario
        target = self.wait(lambda: self.leading(), "fault leader")
        if scenario in ("restart", "snapshot"):
            target = next(node for node in self.nodes if node != target)
        event = {"scenario": scenario, "target": target, "started_ns": time.monotonic_ns() - recorder.origin}
        self.events.append(event)
        if scenario == "transfer":
            successor = next(node for node in self.nodes if node != target)
            self.api(target, "/benchmark/transfer", successor)
            self.wait(lambda: self.leading(exclude=target), "leadership transfer")
        elif scenario == "partition":
            try:
                self.partition(target)
                if self.shutdown.wait(self.args.fault_seconds):
                    return
            finally:
                self.partition(None)
            event["partition_scope"] = "both_directions_all_raft_rpcs_application_http_stays_reachable"
            self.wait(lambda: self.leading(), "leadership after healed partition")
        else:
            before = self.api(target, "/metrics")
            index = (before.get("last_applied") or {}).get("index", 0)
            self.stop(target)
            if self.shutdown.wait(self.args.fault_seconds):
                return
            if scenario == "snapshot":
                leader = self.wait(lambda: self.leading(exclude=target), "snapshot source")
                for operation in range(self.args.snapshot_logs * 2 + 4):
                    self.api(leader, "/write", {"Set": {"key": f"control/{self.run_id}", "value": str(operation)}}, recovery_deadline)
                def purged():
                    metrics = self.api(leader, "/metrics")
                    snapshot = (metrics.get("snapshot") or {}).get("index", 0)
                    purged_index = (metrics.get("purged") or {}).get("index", 0)
                    return snapshot if purged_index > index and snapshot >= purged_index else None
                event["required_snapshot_index"] = self.wait(purged, "leader purges past stopped follower")
            self.start(target)
            self.wait(lambda: self.leading(), "leadership after restart")
            if scenario == "snapshot":
                def installed():
                    metrics = self.api(target, "/metrics")
                    return (metrics.get("snapshot") or {}).get("index", 0) >= event["required_snapshot_index"]
                self.wait(installed, "follower installs checkpoint")
                self.stop(target)
                self.start(target)
                event["checkpoint_restart_checked"] = True
        event["finished_ns"] = time.monotonic_ns() - recorder.origin

    def cas_retry_check(self):
        leader = self.wait(lambda: self.leading(), "CAS check leader")
        key = f"cas/{self.run_id}"
        initial = self.api(leader, "/write", {"Set": {"key": key, "value": "seed"}})
        version = initial["data"]["value"]["version"]
        request = {"CompareAndSet": {"key": key, "expected_version": version, "value": "once"}}
        original = self.api(leader, "/write", request)
        retried = self.api(leader, "/write", request)
        expected = original["data"]["value"]
        if expected is None or expected["value"] != "once" or retried["data"]["value"] is not None:
            raise OracleViolation("CAS retry mutated twice or failed to mutate once")
        for node in self.nodes:
            found = self.api(node, "/follower_read", key)["value"]
            if found != expected:
                raise OracleViolation("CAS result differs across nodes")
        return {"setup_proposals": 1, "acknowledged_raft_proposals": 2, "successful_semantic_mutations": 1,
                "original_log_id": original["log_id"], "retry_log_id": retried["log_id"],
                "scope": "discarded_response_control_retries_same_CAS_not_global_proposal_deduplication"}

    def verify(self):
        leader = self.wait(lambda: self.leading(), "final leader")
        barrier = self.api(leader, "/write", {"Set": {"key": f"control/drain/{self.run_id}", "value": "drained"}})
        boundary = barrier["log_id"]["index"]
        expected = dict(self.recorder.acknowledged)

        def applied():
            return {node: (self.api(node, "/metrics").get("last_applied") or {}).get("index", 0)
                    for node in self.nodes}

        def consistent():
            before = applied()
            if min(before.values()) < boundary:
                return None
            observed = {}
            for key, required in expected.items():
                values = {}
                for node in self.nodes:
                    value = self.api(node, "/follower_read", key)["value"]
                    self.check_value(key, value, required)
                    values[node] = value
                observed[key] = values
            after = applied()
            if before != after or len(set(after.values())) != 1:
                return None
            for key, values in observed.items():
                reference = values[leader]
                if any(value != reference for value in values.values()):
                    raise OracleViolation(f"stable final values differ: {key}")
            return after

        boundaries = self.wait(consistent, "stable final application values", seconds=self.args.drain)
        return {"verified_keys": len(expected), "verified_nodes": list(self.nodes), "applied_boundaries": boundaries,
                "all_acknowledgement_floors_checked": True,
                "duplicate_detection_scope": "retained_client_attempts_and_CAS_control; lost_Set_responses_can_hide_duplicate_proposals"}

    def close(self):
        self.shutdown.set()
        for node in list(self.processes):
            self.stop(node)
        self.http.close_all()
        for log in self.logs:
            log.close()


def workload(cluster):
    args = cluster.args
    recorder = cluster.recorder = Recorder(args)
    workers = []
    generators = []
    failures = queue.Queue()
    stopped = threading.Event()

    def worker(kind, incoming):
        try:
            while True:
                try:
                    item = incoming.get(timeout=0.1)
                except queue.Empty:
                    if stopped.is_set():
                        return
                    continue
                if item is None:
                    return
                operation, scheduled = item
                if time.monotonic() >= recorder.deadline:
                    recorder.outcome(kind, scheduled, None, "pending_at_drain_deadline")
                else:
                    cluster.operation(kind, operation, scheduled)
        except Exception as error:
            failures.put(error)
            stopped.set()

    def generate(kind, rate, incoming, count):
        operation = 0
        while recorder.epoch + operation / rate < recorder.end and not stopped.is_set():
            scheduled = recorder.epoch + operation / rate
            if time.monotonic() >= recorder.end:
                recorder.outcome(kind, scheduled, None, "missed")
                operation += 1
                continue
            time.sleep(max(0, scheduled - time.monotonic()))
            try:
                incoming.put_nowait((operation, scheduled))
                with recorder.lock:
                    recorder.peak_queue[kind] = max(recorder.peak_queue[kind], incoming.qsize())
            except queue.Full:
                recorder.outcome(kind, scheduled, None, "rejected")
            operation += 1
        for _ in range(count):
            while not stopped.is_set():
                try:
                    incoming.put(None, timeout=0.1)
                    break
                except queue.Full:
                    if time.monotonic() >= recorder.deadline:
                        stopped.set()

    for kind, rate, count in [("write", args.write_rate, args.clients), ("read", args.read_rate, args.read_clients)]:
        if rate == 0:
            continue
        incoming = queue.Queue(args.queue_capacity)
        for _ in range(count):
            thread = threading.Thread(target=worker, args=(kind, incoming))
            thread.start()
            workers.append(thread)
        thread = threading.Thread(target=generate, args=(kind, rate, incoming, count))
        thread.start()
        generators.append(thread)
    fault_errors = []
    def fault():
        try:
            cluster.fault()
        except Exception as error:
            fault_errors.append(error)
    control = None
    if args.scenario != "normal":
        control = threading.Thread(target=fault)
        control.start()
    for thread in generators:
        thread.join(max(0, recorder.deadline - time.monotonic()))
    for thread in workers:
        thread.join(max(0, recorder.deadline - time.monotonic()))
    if any(thread.is_alive() for thread in workers + generators):
        stopped.set()
        cluster.http.close_all()
        for thread in workers + generators:
            thread.join(args.request_timeout + 1)
    if any(thread.is_alive() for thread in workers + generators):
        raise TimeoutError("client workers did not stop after the bounded drain")
    if control:
        control.join(max(0, recorder.deadline + 35 - time.monotonic()))
        if control.is_alive():
            raise TimeoutError("recovery control did not finish")
    if fault_errors:
        raise fault_errors[0]
    if not failures.empty():
        raise failures.get()
    if recorder.oracle_errors:
        raise OracleViolation(recorder.oracle_errors[0])
    return recorder.report()


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=BACKENDS, default="wal")
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--attach", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--warmup", type=float, default=5)
    parser.add_argument("--drain", type=float, default=10)
    parser.add_argument("--request-timeout", type=float, default=5)
    parser.add_argument("--write-rate", type=int, default=100)
    parser.add_argument("--read-rate", type=int, default=900)
    parser.add_argument("--clients", type=int, default=16)
    parser.add_argument("--read-clients", type=int, default=32)
    parser.add_argument("--read-on-follower", type=int)
    parser.add_argument("--queue-capacity", type=int, default=1024)
    parser.add_argument("--keys", type=int, default=64)
    parser.add_argument("--payload-bytes", type=int, default=256)
    parser.add_argument("--hot-key-percent", type=int, default=0)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--history-every", type=int, default=128)
    parser.add_argument("--history-capacity", type=int, default=4096)
    parser.add_argument("--port-base", type=int, default=41000)
    parser.add_argument("--scenario", choices=["normal", "transfer", "crash", "partition", "restart", "snapshot"], default="normal")
    parser.add_argument("--fault-after", type=float, default=1)
    parser.add_argument("--fault-seconds", type=float, default=1)
    parser.add_argument("--snapshot-logs", type=int, default=16)
    args = parser.parse_args()
    positive = [args.seconds, args.drain, args.request_timeout,
                args.queue_capacity, args.keys, args.history_capacity, args.snapshot_logs]
    if not all(math.isfinite(value) and value > 0 for value in positive):
        parser.error("durations, workers, capacities, keys and snapshot threshold must be positive")
    nonnegative = [args.write_rate, args.read_rate, args.payload_bytes, args.warmup, args.fault_after, args.fault_seconds, args.history_every]
    if not all(math.isfinite(value) and value >= 0 for value in nonnegative):
        parser.error("rates, payload, warmup, fault timings and sampling must be nonnegative")
    if args.clients < 0 or args.read_clients < 0 or (args.write_rate > 0 and args.clients == 0) or (args.read_rate > 0 and args.read_clients == 0):
        parser.error("each positive rate requires positive workers")
    if not 1 <= args.port_base <= 63532:
        parser.error("port-base must leave room for three node/API/control port ranges")
    if args.snapshot_logs > 65536:
        parser.error("snapshot-logs exceeds the bounded recovery-control limit")
    if args.payload_bytes > 16 * 1024 * 1024:
        parser.error("payload-bytes exceeds 16 MiB")
    if args.write_rate + args.read_rate == 0 or args.keys > 65536 or not 0 <= args.hot_key_percent <= 100:
        parser.error("choose a positive load, at most 65536 keys, and hot-key-percent in 0..100")
    if args.scenario == "snapshot" and args.backend != "rocks":
        parser.error("snapshot recovery requires the persistent RocksDB profile")
    if args.attach and args.scenario in ("crash", "restart", "snapshot"):
        parser.error("process lifecycle scenarios require owned local processes")
    if args.scenario != "normal" and args.fault_seconds > args.seconds - args.fault_after + args.drain:
        parser.error("fault duration must fit within measurement and drain")
    if args.scenario != "normal" and args.fault_after >= args.seconds:
        parser.error("fault-after must fall inside the measurement window")
    return args


def main():
    args = parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    cluster = Cluster(args, args.output)
    report = {"schema_version": 1, "config": {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()},
              "machine": platform.platform(), "nodes": cluster.nodes, "replication": "unary_sequential_http",
              "read_policy": "ReadIndex", "payload_scope": "logical_ID_string_plus_payload_bytes",
              "durability": "WAL sync append; volatile SM, all logs retained" if args.backend == "wal" else "RocksDB sync log flush and atomic sync SM writes"}
    succeeded = False
    try:
        if not args.attach:
            cluster.initialize()
        else:
            cluster.wait(lambda: cluster.leading(), "attached cluster leader")
        if args.read_on_follower is not None and args.read_on_follower not in cluster.nodes:
            raise ValueError("read-on-follower must name a configured node")
        report["commands"] = cluster.commands
        if not args.attach:
            executable = Path(cluster.commands[1][0])
            report["executable_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
        report["resolved_raft_config"] = {node: cluster.api(node, "/benchmark/config") for node in cluster.nodes}
        report["workload"] = workload(cluster)
        report["application_http"] = {node: cluster.api(node, "/benchmark/stats") for node in cluster.nodes}
        report["http_counter_units"] = ["requests", "request_body_bytes", "response_body_bytes"]
        report["http_counter_scope"] = "node lifetime through workload; includes follower authorization /get_linearizer"
        report["verification"] = cluster.verify()
        report["cas_retry_control"] = cluster.cas_retry_check()
        succeeded = True
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        report["scenario_events"] = cluster.events
        if cluster.recorder:
            report["partial_workload"] = cluster.recorder.report()
            report["history_dropped"] = cluster.recorder.dropped
            report["oracle_error_count"] = cluster.recorder.oracle_error_count
            with (args.output / "history.jsonl").open("w") as output:
                for event in cluster.recorder.events:
                    print(json.dumps(event), file=output)
        report["client_http_body_bytes"] = {"sent": cluster.http.sent, "received": cluster.http.received}
        report["success"] = succeeded
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        cluster.close()
    print(json.dumps(report))


if __name__ == "__main__":
    main()
