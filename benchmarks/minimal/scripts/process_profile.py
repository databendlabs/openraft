#!/usr/bin/env python3
"""Run a bounded, paced HTTP workload against durable benchmark nodes backed by WAL or RocksDB.

Spawn three local nodes, or --attach to a JSON manifest containing `nodes` with
id/api_addr/raft_addr (and optional control_addr). All artifacts stay in --output.
Existing HTTP read endpoints use ReadIndex; replication is the unary adapter.
"""

import argparse
import collections
import errno
import hashlib
import http.client
import ipaddress
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

from benchmark_build import BuildCache, BuildRequest, verified_entry
from benchmark_common import DEFAULT_CACHE, HostMonitor, harness_state, machine

ROOT = Path(__file__).resolve().parents[3]
BACKENDS = ("wal", "rocks")
DURABLE_NODE = ROOT / "benchmarks/durable/target/release/bench-durable-node"


class AvailabilityError(RuntimeError):
    def __init__(self, message, status=None):
        super().__init__(message)
        self.status = status


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


class DeadlineSocket(socket.socket):
    def remaining(self):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("absolute HTTP deadline reached")
        self.settimeout(remaining)

    def recv_into(self, buffer, nbytes=0, flags=0):
        self.remaining()
        return super().recv_into(buffer, nbytes, flags)

    def sendall(self, data, flags=0):
        self.remaining()
        return super().sendall(data, flags)


class DeadlineConnection(http.client.HTTPConnection):
    def connect(self):
        address = ipaddress.ip_address(self.host)
        family = socket.AF_INET if address.version == 4 else socket.AF_INET6
        self.sock = DeadlineSocket(family, socket.SOCK_STREAM)
        self.sock.deadline = self.deadline
        self.sock.remaining()
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.sock.connect((str(address), self.port))
        self.sock.remaining()


class Http:
    def __init__(self):
        self.local = threading.local()
        self.lock = threading.Lock()
        self.connections = set()
        self.sent = 0
        self.received = 0
        self.unknown_partial_sends = 0

    def connection(self, endpoint, deadline):
        connections = getattr(self.local, "connections", None)
        if connections is None:
            connections = self.local.connections = {}
        connection = connections.get(endpoint)
        if connection is None:
            connection = DeadlineConnection(*endpoint)
            connections[endpoint] = connection
        connection.timeout = max(0, deadline - time.monotonic())
        connection.deadline = deadline
        if connection.sock:
            connection.sock.deadline = deadline
            connection.sock.remaining()
        with self.lock:
            self.connections.add(connection)
        return connection

    def request(self, address, path, body, deadline):
        if deadline <= time.monotonic():
            raise TimeoutError("logical request deadline reached")
        parsed = urlsplit(address if "://" in address else "http://" + address)
        if parsed.scheme != "http" or not parsed.hostname:
            raise ValueError("node addresses must be HTTP endpoints")
        ipaddress.ip_address(parsed.hostname)
        endpoint = (parsed.hostname, parsed.port or 80)
        connection = self.connection(endpoint, deadline)
        encoded = None if body is None else json.dumps(body, separators=(",", ":")).encode()
        return self.exchange(connection, path, encoded, deadline)

    def send(self, connection, path, encoded):
        method = "GET" if encoded is None else "POST"
        connection.request(method, path, encoded, {"Content-Type": "application/json"})
        with self.lock:
            self.sent += len(encoded or b"")

    def read(self, response):
        chunks = []
        while True:
            chunk = response.read1(65536)
            if not chunk:
                return b"".join(chunks)
            chunks.append(chunk)
            with self.lock:
                self.received += len(chunk)

    def exchange(self, connection, path, encoded, deadline):
        sent = False
        try:
            self.send(connection, path, encoded)
            sent = True
            response = connection.getresponse()
            data = self.read(response)
            if time.monotonic() >= deadline:
                raise TimeoutError("HTTP response completed after its logical deadline")
        except (OSError, http.client.HTTPException):
            self.discard(connection, not sent)
            raise
        if response.status != 200:
            raise AvailabilityError(f"HTTP {response.status}: {data[:300]!r}", status=response.status)
        return json.loads(data)

    def discard(self, connection, partial):
        connection.close()
        endpoint = (connection.host, connection.port)
        del self.local.connections[endpoint]
        with self.lock:
            self.connections.discard(connection)
            self.unknown_partial_sends += int(partial)

    def close_connection(self, connection):
        sock = connection.sock
        try:
            if sock:
                sock.shutdown(socket.SHUT_RDWR)
        except OSError as error:
            if error.errno not in (errno.ENOTCONN, errno.EBADF, errno.ECONNRESET):
                raise
        finally:
            connection.close()

    def close_all(self):
        with self.lock:
            connections = list(self.connections)
            self.connections.clear()
        errors = []
        for connection in connections:
            try:
                self.close_connection(connection)
            except OSError as error:
                errors.append(str(error))
        if errors:
            raise OSError("HTTP close failures: " + "; ".join(errors))


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
        self.epoch = self.origin / 1e9
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
                self.counts[kind]["offered"] += 1
                self.counts[kind]["admitted"] += int(outcome not in ("rejected", "missed"))
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
                           "observed_peak_waiting_requests": self.peak_queue[kind],
                           "operations_per_second": self.counts[kind]["completed_in_window"] / self.args.seconds,
                           **{name: histogram.report() for name, histogram in self.histograms[kind].items()}}
                    for kind in self.counts}


class Cluster:
    def __init__(self, args, output):
        self.args = args
        self.output = output
        self.http = Http()
        self.lifecycle_state()
        self.run_id = uuid.uuid4().hex
        self.nodes = self.attach() if args.attach else self.local_nodes()

    def lifecycle_state(self):
        self.processes = {}
        self.commands = {}
        self.logs = []
        self.leader_id = 1
        self.events = []
        self.recorder = None
        self.runner = None
        self.shutdown = threading.Event()
        self.lifecycle = threading.Lock()
        self.control_errors = collections.deque(maxlen=64)
        self.control_error_count = 0

    def attach(self):
        manifest = json.loads(self.args.attach.read_text())
        nodes = manifest["nodes"]
        if len(nodes) < 3 or len({node["id"] for node in nodes}) != len(nodes):
            raise ValueError("attach manifest needs at least three distinct node IDs")
        for node in nodes:
            if type(node["id"]) is not int or node["id"] <= 0 or not node.get("api_addr") or not node.get("raft_addr"):
                raise ValueError("each attached node requires a positive ID, api_addr and raft_addr")
            if not node.get("control_addr"):
                raise ValueError("attached benchmark nodes require the explicit control listener")
        validate_endpoints(nodes)
        return {node["id"]: node for node in nodes}

    def local_nodes(self):
        return {node: {"id": node, "api_addr": f"127.0.0.1:{self.args.port_base + node}",
                       "raft_addr": f"127.0.0.1:{self.args.port_base + 1000 + node}",
                       "control_addr": f"127.0.0.1:{self.args.port_base + 2000 + node}"}
                for node in (1, 2, 3)}

    def start(self, node):
        if self.args.attach:
            raise ValueError("attach mode does not own node processes")
        executable = self.args.binary or DURABLE_NODE
        directory = self.output / f"node-{node}"
        directory.mkdir(exist_ok=True)
        options = self.nodes[node]
        command = [str(executable), "--backend", self.args.backend, "--id", str(node), "--api-addr", options["api_addr"],
                   "--raft-addr", options["raft_addr"], "--data-dir", str(directory / "data"),
                   "--benchmark-control-addr", options["control_addr"]]
        if self.args.backend == "rocks" and (self.args.snapshot_logs or self.args.scenario == "snapshot"):
            command += ["--snapshot-logs", str(self.args.snapshot_logs or 16), "--snapshot-keep", "0"]
        apply_delay = getattr(self.args, "apply_delay_us", 0)
        if apply_delay:
            command += ["--apply-delay-us", str(apply_delay)]
        self.spawn(node, command, directory / "process.log")
        self.wait(lambda: self.api(node, "/metrics"), f"node {node} API readiness")
        self.wait(lambda: self.http.request(options["control_addr"], "/status", None, time.monotonic() + 1),
                  f"node {node} control readiness")

    def spawn(self, node, command, log_path):
        with self.lifecycle:
            if self.shutdown.is_set():
                raise RuntimeError("benchmark shutdown forbids starting a node")
            existing = self.processes.get(node)
            if existing is not None and existing.poll() is None:
                raise ValueError("benchmark node is already running")
            log = log_path.open("ab")
            self.logs.append(log)
            self.commands[node] = command
            env = dict(os.environ)
            workers = getattr(self.args, "server_workers", None)
            if workers is not None:
                env["TOKIO_WORKER_THREADS"] = str(workers)
            self.processes[node] = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)

    def stop(self, node):
        process = self.processes[node]
        if process.poll() is None:
            process.kill()
        process.wait(timeout=10)

    def api(self, node, path, body=None, deadline=None):
        if self.shutdown.is_set():
            raise RuntimeError("benchmark shutdown forbids new application requests")
        deadline = deadline or time.monotonic() + self.args.request_timeout
        result = self.http.request(self.nodes[node]["api_addr"], path, body, deadline)
        return unwrap(result)

    def wait(self, check, label, seconds=30):
        deadline = time.monotonic() + seconds
        last = None
        while time.monotonic() < deadline:
            if self.shutdown.is_set():
                raise RuntimeError("benchmark wait interrupted by shutdown")
            try:
                result = check()
                if result is not None and result is not False:
                    return result
            except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as error:
                last = str(error)
            self.shutdown.wait(0.02)
        raise TimeoutError(f"{label}: {last}")

    def control_request(self, node, path, body=None):
        try:
            return self.api(node, path, body)
        except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as error:
            self.control_error_count += 1
            self.control_errors.append({"node": node, "path": path, "error": str(error)})
            raise

    def read_reports(self, path):
        return {node: self.wait(lambda: self.control_request(node, path), f"node {node} {path}")
                for node in self.nodes}

    def write_control(self, request):
        def attempt():
            leader = self.leading()
            if leader is None:
                return None
            return self.control_request(leader, "/write", request)
        return self.wait(attempt, "acknowledged administrative write")

    def leading(self, exclude=None, deadline=None):
        deadline = deadline if deadline is not None else time.monotonic() + self.args.request_timeout
        for node in self.nodes:
            if node == exclude:
                continue
            try:
                metrics = self.api(node, "/metrics", deadline=min(deadline, time.monotonic() + 0.3))
            except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError):
                continue
            leader = metrics.get("current_leader")
            if leader in self.nodes and leader != exclude:
                probe = self.api(leader, "/metrics", deadline=min(deadline, time.monotonic() + 0.3))
                if probe.get("state") == "Leader":
                    self.leader_id = leader
                    return leader
        return None

    def initialize(self):
        for node in self.nodes:
            self.start(node)
        self.api(1, "/init", [])
        self.wait(lambda: self.leading(), "initial leadership")
        # The leader accepts a membership change only after it commits the first entry of its term.
        self.wait(lambda: (self.api(1, "/metrics").get("last_applied") or {}).get("index", 0) >= 1, "initial commit")
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
        request = Operation(self, kind, operation, scheduled)
        request.run()

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
        errors = []
        for node, options in self.nodes.items():
            targets = ["*"] if node == isolated else [str(isolated)] if isolated else []
            try:
                self.http.request(options["control_addr"], "/partition", targets, time.monotonic() + 2)
            except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as error:
                errors.append(f"node {node}: {error}")
        if errors:
            raise AvailabilityError("partition control failures: " + "; ".join(errors))

    def fault(self):
        wait = self.recorder.start + self.args.fault_after - time.monotonic()
        if wait > 0 and self.shutdown.wait(wait):
            return
        target = self.wait(lambda: self.leading(), "fault leader")
        if self.args.scenario in ("restart", "snapshot"):
            target = next(node for node in self.nodes if node != target)
        event = {"scenario": self.args.scenario, "target": target,
                 "started_ns": time.monotonic_ns() - self.recorder.origin}
        self.events.append(event)
        handlers = {"transfer": self.transfer_fault, "partition": self.partition_fault}
        handler = handlers.get(self.args.scenario, self.stopped_fault)
        handler(target, event)
        event["finished_ns"] = time.monotonic_ns() - self.recorder.origin

    def replacement(self, target, event):
        successor = self.wait(lambda: self.leading(exclude=target), "majority elects while old leader is unavailable")
        event["replacement_leader"] = successor
        self.recovery_probe(event)

    def transfer_fault(self, target, event):
        successor = next(node for node in self.nodes if node != target)
        self.api(target, "/benchmark/transfer", successor)
        self.replacement(target, event)

    def partition_fault(self, target, event):
        try:
            self.partition(target)
            self.replacement(target, event)
            self.shutdown.wait(self.args.fault_seconds)
        finally:
            self.partition(None)
        event["partition_scope"] = "both_directions_all_raft_rpcs_application_http_stays_reachable"
        self.wait(lambda: self.leading(), "leadership after healed partition")

    def stopped_fault(self, target, event):
        before = self.api(target, "/metrics")
        index = (before.get("last_applied") or {}).get("index", 0)
        self.stop(target)
        if self.args.scenario == "crash":
            self.replacement(target, event)
        if self.shutdown.wait(self.args.fault_seconds):
            return
        if self.args.scenario == "snapshot":
            self.prepare_snapshot(target, index, event)
        self.start(target)
        self.wait(lambda: self.leading(), "leadership after restart")
        if self.args.scenario == "snapshot":
            self.confirm_snapshot(target, event)

    def purged_snapshot(self, leader, index):
        metrics = self.api(leader, "/metrics")
        snapshot = (metrics.get("snapshot") or {}).get("index", 0)
        purged = (metrics.get("purged") or {}).get("index", 0)
        return snapshot if purged > index and snapshot >= purged else None

    def prepare_snapshot(self, target, index, event):
        leader = self.wait(lambda: self.leading(exclude=target), "snapshot source")
        marker_key = f"snapshot-marker/{self.run_id}"
        marker = self.api(leader, "/write", {"Set": {"key": marker_key, "value": self.run_id}})
        event["checkpoint_marker"] = {"key": marker_key, "value": marker["data"]["value"]}
        index = max(index, marker["log_id"]["index"])
        threshold = self.args.snapshot_logs or 16
        for operation in range(threshold * 2 + 4):
            request = {"Set": {"key": f"control/{self.run_id}", "value": str(operation)}}
            self.api(leader, "/write", request, self.recorder.deadline + 30)
        boundary = self.wait(lambda: self.purged_snapshot(leader, index), "leader purges past stopped follower")
        event["required_snapshot_index"] = boundary

    def confirm_snapshot(self, target, event):
        boundary = event["required_snapshot_index"]
        def installed():
            metrics = self.api(target, "/metrics")
            return (metrics.get("snapshot") or {}).get("index", 0) >= boundary
        self.wait(installed, "follower installs checkpoint")
        self.wait(lambda: self.check_local_marker(target, event), "local checkpoint marker and boundary")
        try:
            self.block_incoming(target)
            self.stop(target)
            self.start(target)
            self.http.request(self.nodes[target]["control_addr"], "/partition", ["*"], time.monotonic() + 2)
            self.wait(lambda: self.check_local_marker(target, event), "local checkpoint marker and boundary")
            event["checkpoint_restart_checked"] = "local_marker_and_applied_metadata_while_Raft_repair_blocked"
        finally:
            self.partition(None)

    def block_incoming(self, target):
        for node, options in self.nodes.items():
            if node != target:
                self.http.request(options["control_addr"], "/partition", [str(target)], time.monotonic() + 2)

    def check_local_marker(self, target, event):
        marker = event["checkpoint_marker"]
        found = self.api(target, "/read", marker["key"])["value"]
        metrics = self.api(target, "/metrics")
        applied = (metrics.get("last_applied") or {}).get("index", 0)
        snapshot = (metrics.get("snapshot") or {}).get("index", 0)
        if min(applied, snapshot) < event["required_snapshot_index"]:
            return False
        if found != marker["value"]:
            raise OracleViolation("checkpoint marker did not persist with its applied boundary")
        return True

    def probe_write(self, event):
        deadline = self.recorder.deadline + 30
        leader = self.leading(exclude=event["target"], deadline=deadline)
        if leader is None:
            return None
        event["recovery_probe_attempts"] = event.get("recovery_probe_attempts", 0) + 1
        request = {"Set": {"key": f"control/recovery/{self.run_id}", "value": "available"}}
        try:
            response = self.api(leader, "/write", request, min(deadline, time.monotonic() + self.args.request_timeout))
        except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as error:
            errors = event.setdefault("recovery_probe_errors", [])
            errors.append(str(error))
            del errors[:-8]
            raise
        event["replacement_leader"] = leader
        return response

    def recovery_probe(self, event):
        budget = max(0, self.recorder.deadline + 30 - time.monotonic())
        response = self.wait(lambda: self.probe_write(event), "acknowledged recovery probe", seconds=budget)
        event["recovery_probe_log_id"] = response["log_id"]
        event["recovered_ns"] = time.monotonic_ns() - self.recorder.origin
        event["outage_upper_bound_ns"] = event["recovered_ns"] - event["started_ns"]

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
                "scope": "repeated_acknowledged_CAS_semantic_control_not_lost_response_injection_or_global_proposal_deduplication"}

    def applied_boundaries(self):
        return {node: (self.api(node, "/metrics").get("last_applied") or {}).get("index", 0)
                for node in self.nodes}

    def observed_values(self, expected):
        observed = {}
        for key, required in expected.items():
            values = {}
            for node in self.nodes:
                value = self.api(node, "/follower_read", key)["value"]
                self.check_value(key, value, required)
                values[node] = value
            observed[key] = values
        return observed

    def consistent_values(self, expected, boundary, leader):
        before = self.applied_boundaries()
        if min(before.values()) < boundary:
            return None
        observed = self.observed_values(expected)
        after = self.applied_boundaries()
        if before != after or len(set(after.values())) != 1:
            return None
        for values in observed.values():
            if any(value is not None and value["version"] > after[node] for node, value in values.items()):
                return None
        for key, values in observed.items():
            reference = values[leader]
            if any(value != reference for value in values.values()):
                raise OracleViolation(f"stable final values differ: {key}")
        return after

    def verify(self):
        leader = self.wait(lambda: self.leading(), "final leader")
        request = {"Set": {"key": f"control/drain/{self.run_id}", "value": "drained"}}
        barrier = self.write_control(request)
        leader = self.leader_id
        boundary = barrier["log_id"]["index"]
        acknowledged = dict(self.recorder.acknowledged)
        keys = [f"bench/{self.run_id}/{index}" for index in range(self.args.keys)]
        expected = {key: acknowledged.get(key) for key in keys}
        check = lambda: self.consistent_values(expected, boundary, leader)
        boundaries = self.wait(check, "stable final application values", seconds=self.args.drain)
        return {"verified_keys": len(expected), "verified_nodes": list(self.nodes), "applied_boundaries": boundaries,
                "acknowledged_keys": len(acknowledged), "all_acknowledgement_floors_checked": True,
                "duplicate_detection_scope": "retained_client_attempts_and_CAS_control; lost_Set_responses_can_hide_duplicate_proposals"}

    def stop_nodes(self):
        with self.lifecycle:
            nodes = list(self.processes)
        errors = []
        for node in nodes:
            try:
                self.stop(node)
            except (OSError, subprocess.TimeoutExpired) as error:
                errors.append(f"node {node}: {error}")
        return errors

    def stop_processes(self):
        errors = self.stop_nodes()
        try:
            self.http.close_all()
        except OSError as error:
            errors.append(str(error))
        for log in self.logs:
            try:
                log.close()
            except OSError as error:
                errors.append(str(error))
        if errors:
            raise RuntimeError("benchmark cleanup failures: " + "; ".join(errors))

    def close(self):
        self.shutdown.set()
        try:
            if self.runner:
                self.runner.cancel()
                self.runner.join_stopped()
        finally:
            try:
                self.stop_processes()
            finally:
                if self.runner:
                    self.runner.join_stopped()


class Operation:
    def __init__(self, cluster, kind, number, scheduled):
        self.cluster = cluster
        self.recorder = cluster.recorder
        self.kind = kind
        self.number = number
        self.scheduled = scheduled
        self.key = cluster.key(number)
        self.deadline = min(scheduled + cluster.args.request_timeout, self.recorder.deadline)
        self.invoked = None
        self.required = None
        self.retries = 0
        self.outcome = "timed_out_before_dispatch"

    def metadata(self):
        if self.invoked is None:
            self.invoked, self.required = self.recorder.begin(self.key, self.kind)
        return {"kind": self.kind, "operation": self.number, "key": self.key,
                "invoked_ns": int((self.invoked - self.recorder.epoch) * 1e9), "attempt": self.retries,
                "scheduled_ns": int((self.scheduled - self.recorder.epoch) * 1e9),
                "attempt_invoked_ns": time.monotonic_ns() - self.recorder.origin}

    def destination(self):
        args = self.cluster.args
        if self.kind == "write":
            return self.cluster.leader_id, "/write", {"Set": {"key": self.key, "value": self.cluster.value(self.number)}}
        follower = args.read_on_follower
        target = follower if follower is not None else self.cluster.leader_id
        path = "/follower_read" if follower is not None else "/linearizable_read"
        return target, path, self.key

    def response(self, response, attempt):
        if self.kind == "write":
            value = response["data"]["value"]
            if value is None or value["value"] != self.cluster.value(self.number):
                raise OracleViolation("write response does not match its logical operation")
            self.recorder.acknowledge(self.key, value["version"], value["value"], self.number)
            attempt.update(log_id=response["log_id"], value=value)
        else:
            value = response["value"]
            self.cluster.check_value(self.key, value, self.required)
            attempt.update(value=value, required_version=self.required[0] if self.required else 0)
        self.outcome = "completed"
        attempt.update(outcome=self.outcome, completed_ns=time.monotonic_ns() - self.recorder.origin)
        self.recorder.event(attempt)

    def retry(self, error, attempt):
        uncertain = self.kind == "write"
        attempt.update(outcome="uncertain" if uncertain else "availability_error", error=str(error),
                       completed_ns=time.monotonic_ns() - self.recorder.origin)
        self.recorder.event(attempt)
        with self.recorder.lock:
            self.recorder.attempts[self.kind]["availability_errors"] += 1
            self.recorder.attempts[self.kind]["uncertain_attempts"] += int(uncertain)
        self.retries += 1
        self.outcome = "timed_out_uncertain" if uncertain else "timed_out"
        try:
            self.cluster.leading(deadline=self.deadline)
        except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as discovery:
            attempt["leader_discovery_error"] = str(discovery)
        time.sleep(min(0.01, max(0, self.deadline - time.monotonic())))

    def oracle_error(self, error, attempt):
        self.outcome = "oracle_violation"
        with self.recorder.lock:
            self.recorder.oracle_errors.append(str(error))
            self.recorder.oracle_error_count += 1
        attempt.update(outcome=self.outcome, error=str(error))
        self.recorder.event(attempt, always=True)

    def attempt(self):
        target, path, body = self.destination()
        attempt = self.metadata()
        attempt["target"] = target
        with self.recorder.lock:
            self.recorder.attempts[self.kind]["submitted"] += 1
            self.recorder.attempts[self.kind]["retry_attempts"] += int(self.retries > 0)
        try:
            response = self.cluster.api(target, path, body, self.deadline)
            self.response(response, attempt)
        except OracleViolation as error:
            self.oracle_error(error, attempt)
        except (OSError, http.client.HTTPException, AvailabilityError, TimeoutError) as error:
            self.retry(error, attempt)
            return False
        return True

    def run(self):
        while time.monotonic() < self.deadline:
            if self.attempt():
                break
        if self.outcome != "completed" and time.monotonic() >= self.recorder.deadline:
            self.outcome = "pending_at_drain_deadline"
        self.recorder.outcome(self.kind, self.scheduled, self.invoked, self.outcome)


class Runner:
    def __init__(self, cluster):
        self.cluster = cluster
        self.args = cluster.args
        self.recorder = Recorder(self.args)
        cluster.recorder = self.recorder
        cluster.runner = self
        self.workers = []
        self.generators = []
        self.failures = queue.Queue()
        self.stopped = threading.Event()
        self.control = None

    def worker(self, kind, incoming):
        try:
            self.consume(kind, incoming)
        except Exception as error:
            self.failures.put(error)
            self.stopped.set()
            self.cluster.shutdown.set()

    def consume(self, kind, incoming):
        while True:
            try:
                item = incoming.get(timeout=0.1)
            except queue.Empty:
                if self.stopped.is_set():
                    return
                continue
            if item is None:
                return
            self.dispatch(kind, item)

    def dispatch(self, kind, item):
        operation, scheduled = item
        now = time.monotonic()
        if now >= self.recorder.deadline:
            self.recorder.outcome(kind, scheduled, None, "pending_at_drain_deadline")
        elif now >= scheduled + self.args.request_timeout:
            self.recorder.outcome(kind, scheduled, None, "timed_out_before_dispatch")
        else:
            self.cluster.operation(kind, operation, scheduled)
            if self.recorder.oracle_errors:
                raise OracleViolation(self.recorder.oracle_errors[0])

    def offer(self, kind, operation, scheduled, incoming):
        if time.monotonic() >= self.recorder.end:
            self.recorder.outcome(kind, scheduled, None, "missed")
            return
        if self.stopped.wait(max(0, scheduled - time.monotonic())):
            return
        try:
            incoming.put_nowait((operation, scheduled))
        except queue.Full:
            self.recorder.outcome(kind, scheduled, None, "rejected")
        with self.recorder.lock:
            self.recorder.peak_queue[kind] = max(self.recorder.peak_queue[kind], incoming.qsize())

    def generate(self, kind, rate, incoming, count):
        operation = 0
        while self.recorder.epoch + operation / rate < self.recorder.end and not self.stopped.is_set():
            scheduled = self.recorder.epoch + operation / rate
            self.offer(kind, operation, scheduled, incoming)
            operation += 1
        if time.monotonic() > self.recorder.deadline:
            self.failures.put(TimeoutError("generator accounting exceeded drain deadline"))
        if self.stopped.is_set() and self.recorder.epoch + operation / rate < self.recorder.end:
            self.failures.put(TimeoutError("generator stopped before enumerating its scheduled tail"))
        self.finish_generation(incoming, count)

    def finish_generation(self, incoming, count):
        for _ in range(count):
            while not self.stopped.is_set():
                try:
                    incoming.put(None, timeout=0.1)
                    break
                except queue.Full:
                    if time.monotonic() >= self.recorder.deadline:
                        self.failures.put(TimeoutError("generator could not account its queue before drain"))
                        self.stopped.set()

    def start_class(self, kind, rate, count):
        if rate == 0:
            return
        incoming = queue.Queue(self.args.queue_capacity)
        for _ in range(count):
            thread = threading.Thread(target=self.worker, args=(kind, incoming))
            thread.start()
            self.workers.append(thread)
        thread = threading.Thread(target=self.generate, args=(kind, rate, incoming, count))
        thread.start()
        self.generators.append(thread)

    def fault(self):
        try:
            self.cluster.fault()
        except Exception as error:
            self.failures.put(error)
            self.stopped.set()
            self.cluster.shutdown.set()

    def cancel(self):
        self.stopped.set()
        self.cluster.http.close_all()

    def join_stopped(self):
        deadline = time.monotonic() + self.args.request_timeout + 2 * len(self.cluster.nodes) + 1
        threads = self.generators + self.workers
        if self.control and self.control is not threading.current_thread():
            threads.append(self.control)
        for thread in threads:
            thread.join(max(0, deadline - time.monotonic()))
        alive = [thread.name for thread in threads if thread.is_alive()]
        if alive:
            raise TimeoutError("benchmark threads did not stop: " + ", ".join(alive))

    def raise_failures(self):
        if self.recorder.oracle_errors:
            raise OracleViolation(self.recorder.oracle_errors[0])
        if not self.failures.empty():
            raise self.failures.get()

    def join_clients(self):
        threads = self.generators + self.workers
        for thread in threads:
            thread.join(max(0, self.recorder.deadline - time.monotonic()))
        if any(thread.is_alive() for thread in threads):
            self.stopped.set()
            self.cluster.http.close_all()
            for thread in threads:
                thread.join(self.args.request_timeout + 1)
        if any(thread.is_alive() for thread in threads):
            raise TimeoutError("client workers did not stop after bounded drain")

    def run(self):
        self.start_class("write", self.args.write_rate, self.args.clients)
        self.start_class("read", self.args.read_rate, self.args.read_clients)
        if self.args.scenario != "normal":
            self.control = threading.Thread(target=self.fault)
            self.control.start()
        self.join_clients()
        self.raise_failures()
        if self.control:
            self.control.join(max(0, self.recorder.deadline + 35 - time.monotonic()))
            if self.control.is_alive():
                raise TimeoutError("recovery control did not finish")
        self.raise_failures()
        return self.recorder.report()


def workload(cluster):
    return Runner(cluster).run()


def endpoint_key(address):
    parsed = urlsplit(address if "://" in address else "http://" + address)
    if parsed.scheme != "http" or parsed.path not in ("", "/"):
        raise ValueError("benchmark endpoints require HTTP numeric addresses without a path")
    ip = ipaddress.ip_address(parsed.hostname)
    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:
        ip = ip.ipv4_mapped
    port = parsed.port if parsed.port is not None else 80
    if not 1 <= port <= 65535:
        raise ValueError("benchmark endpoint port must be in 1..65535")
    return str(ip), port


def validate_endpoints(nodes):
    endpoints = set()
    for node in nodes:
        for role in ("api_addr", "raft_addr", "control_addr"):
            endpoint = endpoint_key(node[role])
            if endpoint in endpoints:
                raise ValueError("each node API/Raft/control endpoint must be unique")
            endpoints.add(endpoint)


def measurement_arguments(parser):
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--warmup", type=float, default=5)
    parser.add_argument("--drain", type=float, default=10)
    parser.add_argument("--request-timeout", type=float, default=5)
    parser.add_argument("--history-every", type=int, default=128)
    parser.add_argument("--history-capacity", type=int, default=4096)


def workload_arguments(parser):
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


def process_arguments(parser):
    parser.add_argument("--backend", choices=BACKENDS)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--attach", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--port-base", type=int)
    parser.add_argument("--scenario", choices=["normal", "transfer", "crash", "partition", "restart", "snapshot"], default="normal")
    parser.add_argument("--fault-after", type=float, default=1)
    parser.add_argument("--fault-seconds", type=float, default=1)
    parser.add_argument("--snapshot-logs", type=int)
    parser.add_argument("--apply-delay-us", type=int, default=0,
                        help="delay every apply() call of each owned WAL node's state machine [default: 0]")


def reproducibility_arguments(parser):
    parser.add_argument("--server-workers", type=int, help="Tokio workers per owned node [default: 16]")
    parser.add_argument("--binary-manifest", type=Path, help="reuse a verified immutable durable-node cache entry")
    parser.add_argument("--build", action="store_true", help="build into the immutable cache (default without --binary)")
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--build-timeout", type=float, default=1800)
    parser.add_argument("--lockfile", type=Path)
    parser.add_argument("--toolchain")


def validate_durations(args, parser):
    positive = [args.seconds, args.drain, args.request_timeout, args.queue_capacity, args.keys, args.history_capacity]
    if not all(math.isfinite(value) and value > 0 for value in positive):
        parser.error("durations, capacities and keys must be positive")
    nonnegative = [args.write_rate, args.read_rate, args.payload_bytes, args.warmup,
                   args.fault_after, args.fault_seconds, args.history_every]
    if not all(math.isfinite(value) and value >= 0 for value in nonnegative):
        parser.error("rates, payload, warmup, fault timings and sampling must be nonnegative")
    if args.scenario != "normal" and args.fault_after >= args.seconds:
        parser.error("fault-after must fall inside measurement")
    if args.scenario != "normal" and args.fault_seconds > args.seconds - args.fault_after + args.drain:
        parser.error("fault duration must fit within measurement and drain")


def validate_load(args, parser):
    missing_writers = args.write_rate > 0 and args.clients == 0
    missing_readers = args.read_rate > 0 and args.read_clients == 0
    if args.clients < 0 or args.read_clients < 0 or missing_writers or missing_readers:
        parser.error("each positive rate requires positive workers")
    if args.payload_bytes > 16 * 1024 * 1024:
        parser.error("payload-bytes exceeds 16 MiB")
    if args.write_rate + args.read_rate == 0 or args.keys > 65536 or not 0 <= args.hot_key_percent <= 100:
        parser.error("choose a positive load, at most 65536 keys, and hot-key-percent in 0..100")


def validate_process(args, parser):
    if args.attach and any(value is not None for value in (args.binary, args.port_base, args.snapshot_logs)):
        parser.error("attach mode cannot set binary, port-base or snapshot-logs")
    if args.attach and args.scenario in ("crash", "restart", "snapshot"):
        parser.error("process lifecycle scenarios require owned local processes")
    if not args.attach:
        args.backend = args.backend or "wal"
        if args.port_base is None:
            args.port_base = 41000
    if args.port_base is not None and not 1 <= args.port_base <= 63532:
        parser.error("port-base must leave room for all three port ranges")
    if args.snapshot_logs is not None and not 1 <= args.snapshot_logs <= 65536:
        parser.error("snapshot-logs must be in 1..65536")
    if args.scenario == "snapshot" and args.backend != "rocks":
        parser.error("snapshot recovery requires the persistent RocksDB profile")
    if args.apply_delay_us < 0:
        parser.error("apply-delay-us must be nonnegative")
    if args.apply_delay_us and (args.attach or args.backend != "wal"):
        parser.error("apply-delay-us requires owned WAL nodes")


def validate_reproducibility(args, parser):
    local = [args.server_workers, args.binary_manifest, args.lockfile, args.toolchain]
    if args.attach and (args.build or any(value is not None for value in local)):
        parser.error("attach mode cannot change server workers or build inputs")
    if args.server_workers is not None and args.server_workers <= 0:
        parser.error("server-workers must be positive")
    if not math.isfinite(args.build_timeout) or args.build_timeout <= 0:
        parser.error("build-timeout must be positive")
    if sum(bool(value) for value in [args.build, args.binary, args.binary_manifest]) > 1:
        parser.error("choose one of build, binary or binary-manifest")
    if not args.attach:
        args.server_workers = args.server_workers or 16
        if args.binary is None and args.binary_manifest is None:
            args.build = True


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    measurement_arguments(parser)
    workload_arguments(parser)
    process_arguments(parser)
    reproducibility_arguments(parser)
    args = parser.parse_args()
    validate_durations(args, parser)
    validate_load(args, parser)
    validate_process(args, parser)
    validate_reproducibility(args, parser)
    return args


def metadata(args, cluster):
    config = {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()}
    return {"schema_version": 2, "config": config, "machine": machine(), "harness": harness_state(), "nodes": cluster.nodes,
            "replication": "unary_sequential_http", "read_policy": "ReadIndex",
            "payload_scope": "logical_ID_string_plus_payload_bytes", "commands": cluster.commands, "success": False}


def prepare_binary(cluster, report):
    args = cluster.args
    if args.attach:
        return
    manifest = getattr(args, "binary_manifest", None)
    if manifest is not None:
        captured = json.loads(manifest.read_text())
        build = verified_entry(manifest.parent, captured["inputs"])
    elif getattr(args, "build", False):
        features = ("runtime-stats", "rocks") if args.backend == "rocks" else ("runtime-stats",)
        request = BuildRequest("durable", features=features, lockfile=args.lockfile, toolchain=args.toolchain,
                               crate="benchmarks/durable", binary="bench-durable-node")
        build = BuildCache(args).build(request)
    else:
        report["binary_provenance"] = "build inputs unknown for an explicitly supplied binary"
        return
    args.binary = Path(build["binary"])
    report["build"] = build


def topology_metadata(cluster, report):
    metrics = cluster.read_reports("/metrics")
    report["topology_verification"] = {"declared_to_observed_id": {node: value.get("id") for node, value in metrics.items()}}
    for node, value in metrics.items():
        if type(value.get("id")) is not int or value["id"] != node:
            raise ValueError(f"API endpoint for node {node} reports ID {value.get('id')}")
    initial = {node: cluster.http.request(options["control_addr"], "/status", None,
                                        time.monotonic() + cluster.args.request_timeout)
               for node, options in cluster.nodes.items()}
    report["topology_verification"] = {"ids": list(metrics), "initial_fault_state": initial,
                                       "preexisting_fault_policy": "reject_nonempty"}
    if any(not isinstance(value, list) or value for value in initial.values()):
        raise ValueError("benchmark requires empty initial fault-control state")


def runtime_report(cluster, node):
    try:
        return cluster.api(node, "/benchmark/runtime")
    except AvailabilityError as error:
        if error.status != 404:
            raise
        if not cluster.args.attach:
            raise ValueError("owned binary lacks /benchmark/runtime; rebuild the durable harness") from error
        return {"server_worker_threads": None, "verification": "unknown", "reason": "runtime endpoint unsupported"}


def runtime_metadata(cluster, report):
    runtimes = {node: cluster.wait(lambda: runtime_report(cluster, node), f"node {node} runtime metadata")
                for node in cluster.nodes}
    expected = getattr(cluster.args, "server_workers", None)
    if expected is not None and any(value.get("server_worker_threads") != expected for value in runtimes.values()):
        raise ValueError("effective server worker allocation differs from the requested setting")
    args = cluster.args
    report["server_runtime"] = runtimes
    report["client_threads"] = {"write_workers": args.clients, "read_workers": args.read_clients,
        "arrival_generators": int(args.write_rate > 0) + int(args.read_rate > 0),
        "fault_controller": int(args.scenario != "normal"), "host_sampler": 1,
        "scope": "Python threads; server_worker_threads are per-node Tokio runtime workers"}


def runtime_stats_report(cluster, node):
    try:
        return cluster.api(node, "/benchmark/runtime_stats")
    except AvailabilityError as error:
        if error.status != 404:
            raise
        if not cluster.args.attach:
            raise ValueError("owned binary lacks /benchmark/runtime_stats; rebuild the durable harness") from error
        return None


def runtime_stats(cluster, report):
    report["runtime_stats"] = {node: cluster.wait(lambda: runtime_stats_report(cluster, node), f"node {node} runtime stats")
                               for node in cluster.nodes}
    report["runtime_stats_scope"] = ("per node after the workload; null without the runtime-stats feature; "
        "log_stages_us are microseconds per entry over the last Config::log_stage_capacity log ranges; "
        "apply_call_entries covers every apply() call since the node started")


def server_metadata(cluster, report):
    topology_metadata(cluster, report)
    runtime_metadata(cluster, report)
    report["resolved_raft_config"] = cluster.read_reports("/benchmark/config")
    storage = cluster.read_reports("/benchmark/storage")
    backends = {item["backend"] for item in storage.values()}
    if len(backends) != 1 or (cluster.args.backend is not None and cluster.args.backend not in backends):
        raise ValueError("node storage profile differs from the selected backend or other nodes")
    report["storage"] = storage
    if not cluster.args.attach:
        apply_delay = getattr(cluster.args, "apply_delay_us", 0)
        if any(item.get("apply_delay_us", 0) != apply_delay for item in storage.values()):
            raise ValueError("node apply delay differs from --apply-delay-us")
        executable = Path(cluster.commands[1][0])
        report["executable_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()


def execute_cluster(cluster, report):
    if not cluster.args.attach:
        cluster.initialize()
    else:
        cluster.wait(lambda: cluster.leading(), "attached cluster leader")
    follower = cluster.args.read_on_follower
    if follower is not None and follower not in cluster.nodes:
        raise ValueError("read-on-follower must name a configured node")
    server_metadata(cluster, report)
    report["workload"] = workload(cluster)
    runtime_stats(cluster, report)
    report["application_http"] = cluster.read_reports("/benchmark/stats")
    report["http_counter_units"] = ["requests", "request_body_bytes", "response_body_bytes"]
    report["http_counter_scope"] = "startup through post-workload sample; fully received request bodies and constructed responses; includes follower authorization /get_linearizer"
    report["verification"] = cluster.verify()
    report["cas_retry_control"] = cluster.cas_retry_check()
    report["success"] = True


def execute(cluster, report):
    prepare_binary(cluster, report)
    monitor = HostMonitor(cluster.output / "host.jsonl")
    try:
        with monitor:
            execute_cluster(cluster, report)
    finally:
        report["host"] = monitor.report()


def write_history(cluster, report):
    recorder = cluster.recorder
    if recorder is None:
        return
    report["partial_workload"] = recorder.report()
    report["history_dropped"] = recorder.dropped
    report["oracle_error_count"] = recorder.oracle_error_count
    report["window"] = {"clock": "client_monotonic", "measurement_start_ns": int(cluster.args.warmup * 1e9),
        "measurement_end_ns": int((cluster.args.warmup + cluster.args.seconds) * 1e9),
        "attempt_scope": "whole_workload_including_warmup_and_drain"}
    with (cluster.output / "history.jsonl").open("w") as output:
        for event in recorder.events:
            print(json.dumps(event), file=output)


def write_report(cluster, report):
    report["scenario_events"] = cluster.events
    report["control_errors"] = {"total": cluster.control_error_count, "retained": list(cluster.control_errors)}
    write_history(cluster, report)
    report["client_http_body_bytes"] = {"fully_sent_bodies": cluster.http.sent,
        "observed_response_bytes": cluster.http.received, "unknown_partial_sends": cluster.http.unknown_partial_sends}
    (cluster.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


def finish(cluster, report):
    try:
        cluster.close()
    except Exception as error:
        report["success"] = False
        report["cleanup_error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        write_report(cluster, report)


def main():
    args = parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    cluster = Cluster(args, args.output)
    report = metadata(args, cluster)
    try:
        execute(cluster, report)
    except Exception as error:
        report["error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        finish(cluster, report)
    print(json.dumps(report))


if __name__ == "__main__":
    main()
