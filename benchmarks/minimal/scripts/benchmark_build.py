"""Build immutable revision binaries with captured locks and effective build inputs."""

import contextlib
import dataclasses
import fcntl
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
from pathlib import Path

from benchmark_common import Command, ROOT, digest, file_hash, git, json_bytes, require_success, source_state, write_json

BUILD_ENV = {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS", "CARGO_BUILD_TARGET", "RUSTC",
             "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTUP_TOOLCHAIN", "CARGO_HOME", "RUSTUP_HOME",
             "CC", "CXX", "CFLAGS", "CXXFLAGS", "LDFLAGS", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET",
             "ROCKSDB_LIB_DIR", "ROCKSDB_INCLUDE_DIR", "LIBCLANG_PATH", "CARGO_INCREMENTAL",
             "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_INCREMENTAL"}


@contextlib.contextmanager
def cache_lock(path, timeout):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as output:
        deadline = time.monotonic() + timeout
        while True:
            try:
                fcntl.flock(output, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"cache is busy: {path}")
                time.sleep(0.1)
        try:
            yield
        finally:
            fcntl.flock(output, fcntl.LOCK_UN)


def build_environment(env=None):
    selected = os.environ if env is None else env
    return {key: value for key, value in selected.items() if key != "CARGO_TARGET_DIR"
            and (key in BUILD_ENV or key.startswith(("CARGO_PROFILE_", "CARGO_TARGET_")))}


def cargo_configs(source):
    directories = [Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))]
    directories += [path / ".cargo" for path in reversed([source, *source.parents])]
    files = []
    for directory in directories:
        candidates = [directory / "config", directory / "config.toml"]
        existing = next((path for path in candidates if path.is_file()), None)
        if existing is not None:
            files.append({"path": str(existing), "sha256": file_hash(existing)})
    return files


def tree_hash(root):
    files = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            files[str(path.relative_to(root))] = {"symlink": os.readlink(path)}
        elif path.is_file():
            files[str(path.relative_to(root))] = {"sha256": file_hash(path), "executable": bool(path.stat().st_mode & 0o111)}
    return digest(json_bytes(files))


def build_tools_config(source):
    settings = {}
    for config in cargo_configs(source):
        path = Path(config["path"])
        values = tomllib.loads(path.read_text()).get("build", {})
        for name in ("rustc", "rustc-wrapper", "rustc-workspace-wrapper"):
            if name in values:
                value = values[name]
                if value and "/" in value and not Path(value).is_absolute():
                    value = str(path.parent.parent / value)
                settings[name] = value
    return settings


def tool_identity(value, cwd, env):
    if not value:
        return {"path": None, "sha256": None}
    path = Path(value)
    if "/" in value and not path.is_absolute():
        path = cwd / path
    resolved = shutil.which(str(path), path=env.get("PATH"))
    if resolved is None:
        raise ValueError(f"build tool is not executable: {value}")
    return {"path": str(Path(resolved).absolute()), "resolved_path": str(Path(resolved).resolve()), "sha256": file_hash(resolved)}


def reject_external_links(source):
    root = source.resolve()
    for path in source.rglob("*"):
        if path.is_symlink() and not path.resolve().is_relative_to(root):
            raise ValueError(f"source snapshots do not support external symlinks: {path.relative_to(source)}")


def snapshot(root, revision, directory):
    current = revision is None
    state = source_state(root) if current else {"revision": git(root, "rev-parse", f"{revision}^{{commit}}"),
                                               "dirty": False, "dirty_sha256": None, "untracked": {}}
    archive = directory / "source.tar"
    with archive.open("wb") as output:
        subprocess.run(["git", "archive", state["revision"]], cwd=root, stdout=output, check=True, timeout=30)
    source = directory / "source"
    source.mkdir()
    with tarfile.open(archive) as contents:
        contents.extractall(source, filter="data")
    if current:
        overlay(root, source, state)
    reject_external_links(source)
    return source, state


def overlay(root, source, state):
    names = subprocess.check_output(["git", "diff", "--no-ext-diff", "--name-only", "-z", "HEAD"], cwd=root, timeout=30)
    changed = names.decode().split("\0")
    for name in set(changed) | set(state["untracked"]):
        if not name:
            continue
        original, destination = root / name, source / name
        if destination.exists() or destination.is_symlink():
            destination.unlink()
        if original.exists() or original.is_symlink():
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(original, destination, follow_symlinks=False)
    if source_state(root) != state:
        raise RuntimeError("checkout changed while creating the build snapshot")


@dataclasses.dataclass
class BuildRequest:
    label: str
    revision: str = None
    features: tuple = ()
    lockfile: Path = None
    toolchain: str = None
    crate: str = "benchmarks/minimal"
    binary: str = "bench"


class BuildCache:
    def __init__(self, args, root=ROOT):
        self.args = args
        self.root = Path(root)
        self.cache = args.cache_dir.resolve()
        self.cache.mkdir(parents=True, exist_ok=True)

    def build(self, request):
        directory = Path(tempfile.mkdtemp(prefix="build-", dir=self.cache))
        try:
            source, state = snapshot(self.root, request.revision, directory)
            job = BuildJob(self, request, directory, source, state)
            result = job.run()
        except BaseException:
            print(f"Build inputs and logs retained: {directory}", file=sys.stderr)
            raise
        if directory.exists():
            shutil.rmtree(directory)
        return result


class BuildJob:
    def __init__(self, cache, request, directory, source, state):
        self.cache, self.request = cache, request
        self.directory, self.source, self.state = directory, source, state
        self.crate = source / request.crate
        self.env = dict(os.environ)
        self.features = sorted(set(request.features))
        manifest = tomllib.loads((self.crate / "Cargo.toml").read_text())
        unsupported = set(self.features) - set(manifest.get("features", {}))
        if unsupported:
            raise ValueError(f"{request.label} does not support features {sorted(unsupported)}")

    def command(self, args, name):
        command = Command(args, cwd=self.crate, timeout=self.cache.args.build_timeout, env=self.env)
        output = self.directory / name
        result = command.run(output)
        require_success(result, output)
        return (output / "stdout").read_text()

    def toolchain(self):
        if self.request.toolchain:
            selected = self.request.toolchain
        else:
            cwd = self.root_context()
            selected = subprocess.check_output(["rustup", "show", "active-toolchain"], cwd=cwd, text=True, timeout=30).split()[0]
        self.env["RUSTUP_TOOLCHAIN"] = selected
        tools = self.effective_tools()
        rustc = tools["rustc"]["path"]
        compiler = self.command([rustc, "-vV"], "rustc")
        cargo = self.command(["cargo", "-vV"], "cargo")
        self.execution_tools = tools
        return {"selected": selected, "rustc": compiler, "cargo": cargo, "executables": self.normalized_tools(tools)}

    def normalized_path(self, value):
        if value and Path(value).is_absolute():
            for root in (self.source, self.source.resolve()):
                if Path(value).is_relative_to(root):
                    return "source/" + str(Path(value).relative_to(root))
        return value

    def normalized_tools(self, tools):
        return {name: {key: self.normalized_path(value) if key in ("path", "resolved_path") else value
                       for key, value in identity.items()} for name, identity in tools.items()}

    def environment_inputs(self):
        return {key: self.normalized_path(value) for key, value in build_environment(self.env).items()}

    def effective_tools(self):
        settings = build_tools_config(self.crate)
        tools = {}
        for name in ("rustc", "rustc-wrapper", "rustc-workspace-wrapper"):
            variable = name.upper().replace("-", "_")
            alias = "CARGO_BUILD_" + variable
            value = self.env.get(variable, self.env.get(alias, settings.get(name)))
            if value is None and name == "rustc":
                value = self.command(["rustup", "which", "rustc"], "rustc-path").strip()
            if name == "rustc" and not value:
                raise ValueError("compiler override cannot be empty")
            identity = tool_identity(value, self.crate, self.env)
            self.env[variable] = identity["path"] or ""
            tools[name] = identity
        return tools

    def root_context(self):
        return self.cache.root / self.request.crate if self.request.revision is None else self.crate

    def inputs(self):
        toolchain = self.toolchain()
        return {"cache_schema": 1, "source": self.state, "crate": self.request.crate, "binary": self.request.binary,
                "features": self.features, "default_features": False, "profile": "release", "toolchain": toolchain,
                "environment": self.environment_inputs(), "cargo_configs": self.config_inputs(),
                "configuration_scope": "snapshot/cache ancestors and CARGO_HOME; original checkout parent configs are not inherited",
                "machine": {"system": platform.system(), "architecture": platform.machine()}}

    def config_inputs(self):
        configs = cargo_configs(self.crate)
        for config in configs:
            path = Path(config["path"])
            if path.is_relative_to(self.source):
                config["path"] = "source/" + str(path.relative_to(self.source))
        return configs

    def cargo_flags(self):
        flags = ["--no-default-features"]
        if self.features:
            flags += ["--features", ",".join(self.features)]
        return flags

    def captured_lock(self):
        if self.request.lockfile is not None:
            return self.request.lockfile.resolve()
        current = self.cache.root / self.request.crate / "Cargo.lock"
        return current if self.request.revision is None and current.exists() else None

    def resolve_lock(self, key):
        destination = self.crate / "Cargo.lock"
        supplied = self.captured_lock()
        stored = self.cache.cache / "locks" / key / "Cargo.lock"
        if supplied is not None:
            shutil.copyfile(supplied, destination)
        elif stored.exists():
            shutil.copyfile(stored, destination)
        else:
            self.command(["cargo", "generate-lockfile"], "resolve-lock")
            stored.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(destination, stored)
        return {"sha256": file_hash(destination), "origin": str(supplied or stored)}

    def run(self):
        inputs = self.inputs()
        resolution_key = digest(json_bytes(inputs))
        with cache_lock(self.cache.cache / "locks" / f"{resolution_key}.lock", self.cache.args.build_timeout):
            lock = self.resolve_lock(resolution_key)
            inputs["lockfile_sha256"] = lock["sha256"]
            inputs["source_tree_sha256"] = tree_hash(self.source)
            key = digest(json_bytes(inputs))
            entry = self.cache.cache / "binaries" / key
            if entry.exists():
                self.verify_inputs(inputs)
                return verified_entry(entry, inputs)
            return self.compile(entry, inputs, lock)

    def compile(self, entry, inputs, lock):
        self.env["CARGO_TARGET_DIR"] = str(self.cache.cache / "targets" / digest(json_bytes(inputs)))
        flags = ["cargo", "build", "--release", "--locked", "--bin", self.request.binary,
                 "--message-format=json-render-diagnostics", *self.cargo_flags()]
        output = self.command(flags, "compile")
        executable = artifact_executable(output, self.request.binary)
        self.verify_inputs(inputs)
        destination = self.directory / "bin"
        destination.mkdir()
        shutil.copy2(executable, destination / self.request.binary)
        manifest = {"inputs": inputs, "lockfile": lock, "binary_sha256": file_hash(executable), "command": flags,
                    "compiled_features": artifact_features(output), "source_directory": str(entry / "source"),
                    "target_directory": self.env["CARGO_TARGET_DIR"], "execution_tool_paths": self.execution_tools}
        write_json(self.directory / "manifest.json", manifest)
        entry.parent.mkdir(parents=True, exist_ok=True)
        self.directory.rename(entry)
        return verified_entry(entry, inputs)

    def verify_inputs(self, inputs):
        if file_hash(self.crate / "Cargo.lock") != inputs["lockfile_sha256"]:
            raise RuntimeError("Cargo changed the captured lockfile")
        if self.config_inputs() != inputs["cargo_configs"]:
            raise RuntimeError("Cargo configuration changed during the build")
        if tree_hash(self.source) != inputs["source_tree_sha256"]:
            raise RuntimeError("source snapshot changed during the build")
        for identity in inputs.get("toolchain", {}).get("executables", {}).values():
            path = identity["path"]
            if path and path.startswith("source/"):
                path = self.source / path.removeprefix("source/")
            if path and file_hash(path) != identity["sha256"]:
                raise RuntimeError("compiler or wrapper changed during the build")
        if self.request.revision is None and source_state(self.cache.root) != self.state:
            raise RuntimeError("checkout changed during the build; binary was not cached")


def artifact_executable(output, name):
    for line in output.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message.get("target", {}).get("name") == name:
            if message.get("executable"):
                return Path(message["executable"])
    raise ValueError(f"Cargo did not report an executable for {name}")


def artifact_features(output):
    features = {}
    for line in output.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact":
            features[message["package_id"]] = message.get("features", [])
    return features


def verified_entry(entry, inputs):
    manifest = json.loads((entry / "manifest.json").read_text())
    binary = entry / "bin" / inputs["binary"]
    lock = entry / "source" / inputs["crate"] / "Cargo.lock"
    if manifest["inputs"] != inputs or file_hash(binary) != manifest["binary_sha256"]:
        raise ValueError(f"cache input/binary mismatch: {entry}")
    if file_hash(lock) != inputs["lockfile_sha256"]:
        raise ValueError(f"cache lockfile mismatch: {entry}")
    if tree_hash(entry / "source") != inputs["source_tree_sha256"]:
        raise ValueError(f"cache source snapshot mismatch: {entry}")
    return {"binary": str(binary), "manifest": str(entry / "manifest.json"), "metadata": manifest}
