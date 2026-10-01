#!/usr/bin/env python3
"""Opt-in self-service template builds; execute on the isolated Linux host.

Two disjoint sandproof-server pools are required: private builder and runtime.
Only the guest executes BuildKit/umoci/user input. No host container socket,
archive unpack, mount or user command. Exposes Sandproof extension endpoints,
not the native E2B template-build API. See docs/module-contracts.md.
"""
import argparse
import hmac
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import shutil
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import urllib.error
import urllib.parse
import urllib.request

if __package__:
    from .template_build_store import BuildError, BuildStore, MIB, TERMINAL, durable_write
    from .template_stage import file_hash, private_directory, stage
    from .template_inputs import platform_dns_servers
    from .template_build_config import validate_capacity, HOST_MEMORY_OVERHEAD_MIB
else:
    from template_build_store import BuildError, BuildStore, MIB, TERMINAL, durable_write
    from template_stage import file_hash, private_directory, stage
    from template_inputs import platform_dns_servers
    from template_build_config import validate_capacity, HOST_MEMORY_OVERHEAD_MIB


def protected_json(path, maximum=65536):
    path = Path(path)
    if path.resolve(strict=True) != path or not path.is_file():
        raise ValueError("protected regular config required")
    meta = path.stat()
    if meta.st_uid != os.geteuid() or meta.st_mode & 0o077 or meta.st_size > maximum:
        raise ValueError("private owned bounded config required")
    return json.loads(path.read_text())


def configuration(path):
    config = protected_json(path)
    if set(config) != {"root", "bind", "runtime_config", "builder_config", "tools", "limits", "dns_servers"}:
        raise ValueError("invalid build service config")
    runtime, builder = protected_json(config["runtime_config"]), protected_json(config["builder_config"])
    if (not runtime.get("template_registry") or runtime.get("internal_build_vms")
            or not builder.get("internal_build_vms") or builder.get("template_registry")
            or not builder.get("host_resources") or not builder.get("templates")
            or not builder.get("public_ipv4_enabled") or len(builder["credentials"]) != 1):
        raise ValueError("explicit runtime registry and private bounded builder pool required")
    for field in ("namespace", "database", "node_state_dir", "chroot_base", "bind"):
        if runtime[field] == builder[field]:
            raise ValueError("runtime/build pools overlap: " + field)
    if builder["credentials"][0]["secret"] in {c["secret"] for c in runtime["credentials"]}:
        raise ValueError("private builder credential must not be a runtime user credential")
    r_range = range(runtime["uid_base"], runtime["uid_base"] + runtime["max_vm_id"] + 1)
    b_range = range(builder["uid_base"], builder["uid_base"] + builder["max_vm_id"] + 1)
    if max(r_range.start, b_range.start) < min(r_range.stop, b_range.stop):
        raise ValueError("runtime/build UID pools overlap")
    r_cgroup = Path(runtime["host_resources"]["cgroup_parent"]).resolve(strict=True)
    b_cgroup = Path(builder["host_resources"]["cgroup_parent"]).resolve(strict=True)
    if r_cgroup == b_cgroup or r_cgroup in b_cgroup.parents or b_cgroup in r_cgroup.parents:
        raise ValueError("runtime/build cgroup pools overlap")
    root = Path(config["root"])
    private_directory(root)
    registry = Path(runtime["template_registry"]["root"])
    for directory in (registry, registry / "inbox", registry / "artifacts"):
        private_directory(directory)
    if root == registry or root in registry.parents or registry in root.parents:
        raise ValueError("build journal and registry must be disjoint")
    # Budget protection applies to the shared disk on which VM copies and
    # candidate copies are actually allocated; reject cross-filesystem setups.
    if len({p.stat().st_dev for p in (root, registry, Path(runtime["chroot_base"]), Path(builder["chroot_base"]))}) != 1:
        raise ValueError("shared budget filesystem required")
    limits = config["limits"]
    required = {"max_cpu", "max_memory_mib", "max_disk_mib", "timeout_seconds", "max_jobs", "max_jobs_per_tenant",
                "max_pending", "max_pending_per_tenant", "workspace_bytes", "workspace_bytes_per_tenant",
                "max_input_bytes", "max_expanded_bytes", "max_entries", "max_log_bytes"}
    if set(limits) != required or any(type(v) is not int or v <= 0 for v in limits.values()):
        raise ValueError("invalid independent build limits")
    if (limits["max_cpu"] > 255 or limits["max_memory_mib"] > 16384 or limits["max_disk_mib"] > 32768
            or limits["timeout_seconds"] > 3600 or limits["max_input_bytes"] > 8 * 1024 * MIB
            or limits["max_expanded_bytes"] > 32 * 1024 * MIB or limits["max_entries"] > 200000
            or limits["max_log_bytes"] > 16 * MIB or limits["max_jobs"] > 10000):
        raise ValueError("build limits exceed worker protocol")
    if (limits["max_memory_mib"] < 128 or limits["max_disk_mib"] < 512
            or limits["max_input_bytes"] < 512 or limits["max_expanded_bytes"] < 10240):
        raise ValueError("build limits cannot represent a valid worker request")
    for tenant, global_field in (("max_jobs_per_tenant", "max_jobs"), ("max_pending_per_tenant", "max_pending"),
                                 ("workspace_bytes_per_tenant", "workspace_bytes")):
        if limits[tenant] > limits[global_field]:
            raise ValueError("tenant budget exceeds global budget")
    tools = config["tools"]
    if set(tools) != {"buildctl", "buildkitd", "buildkit-runc", "umoci", "envd", "busybox", "regctl"}:
        raise ValueError("fixed tool profile required")
    for spec in tools.values():
        path = Path(spec["path"])
        if path.resolve(strict=True) != path or not path.is_file() or file_hash(path) != spec["sha256"]:
            raise ValueError("tool fingerprint mismatch")
    platform_dns_servers(config["dns_servers"])
    if not re.fullmatch(r"127\.0\.0\.1:[0-9]{1,5}", config["bind"]):
        raise ValueError("loopback build listener required")
    for settings in (runtime, builder):
        if not settings["bind"].startswith("127.0.0.1:"):
            raise ValueError("loopback controllers required")
    if config["bind"] in (runtime["bind"], builder["bind"]):
        raise ValueError("build listener conflicts with a controller")
    catalog = builder["templates"]
    base_id = catalog["aliases"].get("base", "base")
    bases = [entry for entry in catalog["templates"] if entry["template_id"] == base_id]
    if len(bases) != 1 or bases[0]["owner"] not in (None, builder["credentials"][0]["tenant"]):
        raise ValueError("builder base must resolve to an authorized immutable template")
    base = bases[0]
    limits["builder_disk_bytes"] = validate_capacity(runtime, builder, limits, base,
        kernel_bytes=Path(runtime["kernel"]).stat().st_size,
        largest_tool_bytes=max(Path(spec["path"]).stat().st_size for spec in tools.values()))
    if Path(base["rootfs"]["path"]).stat().st_size != limits["builder_disk_bytes"]:
        raise ValueError("builder base disk size differs from its immutable descriptor")
    limits["runtime_cleanup_reserve_bytes"] = runtime["cleanup_reserve_bytes"] + builder["cleanup_reserve_bytes"]
    limits["artifact_margin_bytes"] = Path(runtime["kernel"]).stat().st_size + limits["max_log_bytes"] + 64 * MIB
    config["runtime"], config["builder"] = runtime, builder
    config["binding"] = hashlib.sha256(json.dumps(config, sort_keys=True).encode()).hexdigest()
    return config


class Backend:
    def __init__(self, config):
        if importlib.metadata.version("e2b") != "2.51.0":
            raise ValueError("fixed E2B SDK 2.51.0 required")
        self.config = config
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.runtime_keys = {c["tenant"]: c["secret"] for c in config["runtime"]["credentials"]}
        self.builder_key = config["builder"]["credentials"][0]["secret"]

    def request(self, pool, method, path, body=None, *, owner=None, timeout=60):
        key = self.builder_key if pool == "builder" else self.runtime_keys[owner]
        encoded = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request("http://" + self.config[pool]["bind"] + path, data=encoded, method=method,
            headers={"X-API-Key": key, "Content-Type": "application/json"})
        try:
            with self.opener.open(request, timeout=timeout) as response:
                data = response.read(65537)
                if len(data) > 65536:
                    raise BuildError(502, "ControllerResponseTooLarge")
                return json.loads(data) if data else None
        except urllib.error.HTTPError as error:
            if error.code == 404:
                raise BuildError(404, "NotFound") from None
            raise BuildError(error.code, "ControllerRejectedOrUnknown") from None

    def vm(self, row, create=False):
        request = json.loads(row["request"])
        body = {"timeout": request["timeoutSeconds"] + 60, "allowInternetAccess": request["allowInternetAccess"]} if create else None
        timeout = 60
        if create:
            # Private create includes copying the bounded builder disk. Its
            # wait belongs to the original build deadline, not a 60s RPC cap.
            timeout = min(request["timeoutSeconds"], row["deadline"] - time.time())
            if timeout <= 0:
                raise BuildError(408, "BuildDeadlineExceeded")
        return self.request("builder", "POST" if create else "GET", "/sandproof/internal-build-vms/" + row["id"], body,
                            timeout=timeout)

    def cleanup(self, row):
        try:
            vm = self.vm(row)
        except BuildError as error:
            # No durable admission is conclusive only because the private
            # API commits its build anchor before any host I/O.
            if error.code == 404:
                return True
            raise
        if vm["cleanupComplete"]:
            return True
        try:
            self.request("builder", "DELETE", "/sandboxes/" + vm["sandboxID"])
        except BuildError:
            pass  # A kill timeout is not evidence of reclaim.
        return self.vm(row)["cleanupComplete"]

    def build(self, store, row):
        from e2b import Sandbox, CommandExitException, FileNotFoundException
        from e2b.connection_config import ConnectionConfig
        from packaging.version import Version
        vm = self.vm(row, create=True)
        session = vm.get("session")
        if session is None:
            raise BuildError(502, "BuildVmLaunchUnknown")
        url = "http://" + self.config["builder"]["bind"]
        connection = ConnectionConfig(api_url=url, sandbox_url=url,
            domain=self.config["builder"]["domain"], api_key=self.builder_key, retries=0, request_timeout=30,
            extra_sandbox_headers={"X-Access-Token": session["envdAccessToken"]})
        sandbox = Sandbox(sandbox_id=session["sandboxID"], sandbox_domain=self.config["builder"]["domain"],
            envd_version=Version(session["envdVersion"]), envd_access_token=session["envdAccessToken"], connection_config=connection)
        request = json.loads(row["request"])
        directory = store.directory(row["id"])
        def command(value, timeout=30):
            left = int(row["deadline"] - store.clock())
            if left <= 0:
                raise BuildError(408, "BuildDeadlineExceeded")
            return sandbox.commands.run(value, user="root", timeout=min(timeout, left))
        command("mkdir -p /input/tools /output")
        profile = {}
        for name, spec in self.config["tools"].items():
            if name == "regctl" and request["kind"] != "oci":
                continue
            with Path(spec["path"]).open("rb") as incoming:
                sandbox.files.write("/input/tools/" + name, incoming, user="root")
            profile[name] = {"path": "/input/tools/" + name, "sha256": spec["sha256"]}
        command("chmod 755 /input/tools/*")
        for name in ("template_worker.py", "template_inputs.py", "template_fetch.py"):
            with Path(__file__).with_name(name).open("rb") as incoming:
                sandbox.files.write("/input/" + name, incoming, user="root")
        sandbox.files.write("/input/profile.json", json.dumps(profile), user="root")
        sandbox.files.write("/etc/resolv.conf", "".join("nameserver " + v + "\n" for v in self.config["dns_servers"]), user="root")
        if request["kind"] == "dockerfile":
            if file_hash(directory / "source.tar") != row["input_hash"]:
                raise BuildError(502, "InputDigestMismatch")
            with (directory / "source.tar").open("rb") as incoming:
                sandbox.files.write("/input/source", incoming, user="root")
        else:
            sandbox.files.write("/input/source", json.dumps({"image": request["image"]}), user="root")
        worker = {"kind": "dockerfile" if request["kind"] == "dockerfile" else "oci-reference",
            "disk_mib": request["resources"]["diskMiB"], "min_free_mib": request["resources"]["minFreeMiB"],
            "max_input_bytes": store.limits["max_input_bytes"], "max_expanded_bytes": store.limits["max_expanded_bytes"],
            "max_entries": store.limits["max_entries"], "max_log_bytes": store.limits["max_log_bytes"],
            "timeout_seconds": max(1, int(row["deadline"] - store.clock()))}
        worker["dns_servers"] = self.config["dns_servers"]
        sandbox.files.write("/input/request.json", json.dumps(worker), user="root")
        boot = command("cat /proc/sys/kernel/random/boot_id").stdout.strip()
        if not re.fullmatch(r"[0-9a-f-]{36}", boot):
            raise BuildError(502, "InvalidBuildVmIdentity")
        worker_error_log = None
        try:
            command("python3 /input/template_worker.py --work /output/job --source /input/source --request /input/request.json --profile /input/profile.json --guest-boot-id " + boot, worker["timeout_seconds"])
        except CommandExitException as error:
            # Guards and conversion checks may fail without writing tool logs,
            # even when status.json exists. Retain only bounded guest output;
            # never stringify an SDK exception with host connection context.
            maximum = min(65536, store.limits['max_log_bytes'])
            # Python's final exception line is at the end of stderr. Verbose
            # stdout must not displace it; byte slicing stays UTF-8 agnostic.
            stderr = (error.stderr or '')[-maximum:].encode()[-maximum:]
            stdout = (error.stdout or '')[:maximum].encode()[:maximum - len(stderr)]
            worker_error_log = stdout + stderr
        # Read only opaque bytes, never mount/unpack guest output on the host.
        def download(name, limit, exact=None, *, sparse=False):
            with sandbox.files.read("/output/job/" + name, format="stream", user="root", request_timeout=max(1, row["deadline"] - store.clock())) as incoming:
                return durable_write(directory / name, incoming, limit, exact=exact, sparse=sparse)
        try:
            download("status.json", 128 * 1024)
        except FileNotFoundException:
            if worker_error_log is None:
                raise
            durable_write(directory / 'build.log', [worker_error_log], store.limits['max_log_bytes'])
            raise BuildError(422, 'WorkerPreflightFailed') from None
        status = json.loads((directory / "status.json").read_text())
        total_log = bytearray()
        for name in ("build.log", "daemon.log"):
            try:
                with sandbox.files.read("/output/job/" + name, format="stream", user="root", request_timeout=30) as incoming:
                    for block in incoming:
                        if len(total_log) + len(block) > store.limits["max_log_bytes"]:
                            raise BuildError(502, "BuildLogLimit")
                        total_log.extend(block)
            except Exception as error:
                if isinstance(error, BuildError):
                    raise
                # A failed worker can exit before opening either tool log.
                if not isinstance(error, FileNotFoundException) or (name == "build.log" and worker_error_log is None):
                    raise
        if worker_error_log is not None:
            # Give the actual failure priority over the tail of verbose tool
            # output. Diagnostic bytes share, rather than extend, the log cap.
            marker = b"\n[worker command failed]\n"
            if store.limits["max_log_bytes"] < 2 * len(marker):
                marker = b""
            room = store.limits["max_log_bytes"] - len(marker)
            diagnostic = marker + worker_error_log[-room:]
            del total_log[max(0, store.limits["max_log_bytes"] - len(diagnostic)):]
            total_log.extend(diagnostic)
        durable_write(directory / "build.log", [total_log], store.limits["max_log_bytes"])
        if worker_error_log is not None and status.get("state") == "candidate":
            raise BuildError(422, "WorkerFailed:CommandExit")
        if status.get("state") != "candidate":
            failure = status.get("error_type")
            raise BuildError(422, "WorkerFailed:" + (failure if failure in {"InvalidTemplate", "TimeoutError", "RuntimeError"} else "BuildError"))
        _, digest = download("candidate.ext4", request["resources"]["diskMiB"] * MIB,
                             request["resources"]["diskMiB"] * MIB, sparse=True)
        if digest != status.get("rootfs_sha256"):
            raise BuildError(502, "CandidateDigestMismatch")
        return status

    def publish(self, store, row):
        # A previous POST may have been accepted despite a lost response.
        # Continue observing that identity even after the build deadline; only
        # a definitive absence permits staging and a new publication request.
        try:
            return self.request("runtime", "GET", "/sandproof/template-publications/" + row["id"], owner=row["owner"])
        except BuildError as error:
            if error.code != 404:
                raise
        if store.clock() >= row["deadline"]:
            raise BuildError(408, "BuildDeadlineExceeded")
        config = self.config["runtime"]
        request = json.loads(row["request"])
        status = json.loads(row["candidate"])
        resources = request["resources"]
        try:
            candidate = stage(Path(config["template_registry"]["root"]), Path(config["kernel"]),
                store.directory(row["id"]) / "candidate.ext4", status, row["owner"],
                resources["cpuCount"], resources["memoryMiB"], resources["memoryMiB"] + HOST_MEMORY_OVERHEAD_MIB, candidate_id=row["id"])
        except ValueError:
            raise BuildError(400, "CandidateStagingRejected") from None
        remaining = row["deadline"] - store.clock()
        if remaining <= 0:
            raise BuildError(408, "BuildDeadlineExceeded")
        return self.request("runtime", "POST", "/sandproof/template-publications", {"candidateID": candidate},
                            owner=row["owner"], timeout=remaining)

    def discard_unpublished(self, row):
        # Call only after a definitive pre-admission refusal. A publication
        # that exists belongs to the registry budget, never this cleanup.
        try:
            self.request("runtime", "GET", "/sandproof/template-publications/" + row["id"], owner=row["owner"])
            return False
        except BuildError as error:
            if error.code != 404:
                return False
        path = Path(self.config["runtime"]["template_registry"]["root"]) / "inbox" / row["id"]
        if path.exists():
            private_directory(path)
            shutil.rmtree(path)  # exact supervisor-generated candidate identity
        return True


class Supervisor:
    def __init__(self, store, backend):
        self.store, self.backend = store, backend
        self.recovered = False

    def recover(self):
        # Single process lease. Never rerun unknown user instructions, even
        # when a VM is still live. The same identity is observed and reclaimed.
        for row in self.store.rows():
            if row["state"] in TERMINAL:
                self.remove_inputs(row["id"])
            if row["state"] == "Building":
                self.store.change(row["id"], "Cleaning", failure="InterruptedBuild")
        self.recovered = True

    def finish(self, row):
        job = row["id"]
        try:
            if not self.backend.cleanup(row):
                self.store.change(job, "CleanupUnknown", failure=row["failure"])
                return
        except Exception:
            self.store.change(job, "CleanupUnknown", failure=row["failure"])
            return
        failure = row["failure"]
        if row["candidate"] and not failure and self.store.clock() >= row["deadline"]:
            failure = "BuildDeadlineExceeded"
        if row["candidate"] and not failure:
            self.store.change(job, "Publishing")
        else:
            # Only exact, confirmed cleanup releases reservation and input.
            self.remove_inputs(job)
            self.store.change(job, "Failed", failure=failure or "InterruptedBuild", release=True)

    def remove_inputs(self, job):
        directory = self.store.directory(job)
        for name in ("source.tar", "source.partial", "candidate.ext4", "candidate.partial", "status.json", "status.partial"):
            (directory / name).unlink(missing_ok=True)
        descriptor = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)

    def tick(self):
        if not self.recovered:
            self.recover()
        for row in self.store.rows():
            job, state = row["id"], row["state"]
            if state in TERMINAL:
                continue
            if state in ("AwaitingInput", "Queued") and self.store.clock() >= row["deadline"]:
                with self.store.lock:
                    if job in self.store.uploads:
                        continue
                    self.remove_inputs(job)
                    self.store.change(job, "Failed", failure="BuildDeadlineExceeded", release=True)
                continue
            if state == "Queued":
                self.store.change(job, "Building")  # precedes private create I/O
                try:
                    status = self.backend.build(self.store, row)
                    self.store.change(job, "Cleaning", candidate=status)
                except Exception as error:
                    reason = error.reason if isinstance(error, BuildError) else "BuildOrTransportFailed:" + type(error).__name__[:64]
                    self.store.change(job, "Cleaning", failure=reason)
                self.finish(self.store.get(job))
            elif state in ("Cleaning", "CleanupUnknown"):
                self.finish(row)
            elif state == "Publishing":
                try:
                    publication = self.backend.publish(self.store, row)
                except Exception as error:
                    expired = (isinstance(error, BuildError) and error.code == 408
                               and error.reason == "BuildDeadlineExceeded")
                    if isinstance(error, BuildError) and (error.code in (400, 409, 429) or expired):
                        # These rejections occur before admission; no running
                        # validation VM is hidden behind a refused publication.
                        if self.backend.discard_unpublished(row):
                            self.remove_inputs(job)
                            self.store.change(job, "Failed", failure="BuildDeadlineExceeded" if expired
                                              else "PublicationRejected", release=True)
                    # Publishing is idempotent on the immutable candidate ID.
                    # Unknown publication never rebuilds or allocates a new ID.
                    continue
                if publication["state"] == "ready":
                    if not publication.get("template"):
                        continue
                    self.remove_inputs(job)
                    self.store.change(job, "Ready", result=publication["template"], release=True)
                elif publication["state"] == "failed":
                    self.remove_inputs(job)
                    self.store.change(job, "Failed", failure="PublicationFailed:" + str(publication.get("failure")), release=True)


class BuildHTTP(ThreadingHTTPServer):
    daemon_threads = True
    def __init__(self, address, store, credentials):
        self.store, self.credentials = store, credentials
        self.slots = threading.BoundedSemaphore(8)
        super().__init__(address, Handler)

    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            request.close()
            return
        try:
            super().process_request(request, address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"
    def log_message(self, *_args):
        pass  # No credentials, Dockerfile, request path or guest logs in host logs.

    def handle(self):
        self.request.settimeout(30)
        super().handle()

    def operation(self):
        key = self.headers.get("X-API-Key", "")
        owner = next((c["tenant"] for c in self.server.credentials if hmac.compare_digest(key, c["secret"])), None)
        if owner is None:
            raise BuildError(401, "Unauthorized")
        parsed = urllib.parse.urlsplit(self.path)
        parts = parsed.path.split("/")
        store = self.server.store
        def length(maximum):
            value = self.headers.get("Content-Length")
            if self.headers.get("Transfer-Encoding") or value is None:
                raise BuildError(411, "KnownContentLengthRequired")
            if not re.fullmatch(r"[0-9]{1,12}", value) or not 1 <= int(value) <= maximum:
                raise BuildError(413, "RequestTooLarge")
            return int(value)
        if self.command == "POST" and parsed.path == "/sandproof/template-builds" and not parsed.query:
            size = length(65536)
            data = self.rfile.read(size)
            if len(data) != size:
                raise BuildError(400, "IncompleteRequest")
            return 202, store.submit(owner, json.loads(data), self.headers.get("Idempotency-Key"))
        if len(parts) not in (4, 5) or parts[1:3] != ["sandproof", "template-builds"]:
            raise BuildError(404, "NotFound")
        job = parts[3]
        store.get(job, owner)  # authorize before streaming any user material
        if self.command == "GET" and len(parts) == 4 and not parsed.query:
            return 200, store.query(job, owner)
        if self.command == "GET" and len(parts) == 5 and parts[4] == "logs":
            query = urllib.parse.parse_qs(parsed.query, strict_parsing=True)
            if set(query) - {"offset", "limit"} or any(len(v) != 1 for v in query.values()):
                raise BuildError(400, "InvalidLogRange")
            return 200, store.logs(job, owner, int(query.get("offset", ["0"])[0]), int(query.get("limit", ["65536"])[0]))
        if self.command == "PUT" and len(parts) == 5 and parts[4] == "context" and not parsed.query:
            size = length(store.limits["max_input_bytes"])
            def blocks():
                remaining = size
                # An upload cannot hold a job beyond its original deadline.
                deadline = store.get(job, owner)["deadline"]
                while remaining:
                    if store.clock() >= deadline:
                        raise BuildError(408, "BuildDeadlineExceeded")
                    block = self.rfile.read(min(65536, remaining))
                    if not block:
                        break
                    remaining -= len(block)
                    yield block
            store.upload(job, owner, blocks(), size)
            return 200, store.query(job, owner)
        raise BuildError(404, "NotFound")

    def respond(self):
        try:
            code, result = self.operation()
        except BuildError as error:
            code, result = error.code, {"error": error.reason}
        except (ValueError, KeyError, TypeError):
            code, result = 400, {"error": "InvalidRequest"}
        except Exception:
            code, result = 503, {"error": "BuildServiceUnavailable"}
        data = json.dumps(result).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_POST = respond
    do_GET = respond
    do_PUT = respond


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "linux" or os.geteuid() != 0:
        raise SystemExit("isolated root Linux host required")
    os.umask(0o077)
    config = configuration(args.config)
    store = BuildStore(config["root"], config["limits"], binding=config["binding"])
    supervisor = Supervisor(store, Backend(config))
    supervisor.recover()
    host, port = config["bind"].split(":")
    server = BuildHTTP((host, int(port)), store, config["runtime"]["credentials"])
    stop = threading.Event()
    def work():
        while not stop.is_set():
            try:
                supervisor.tick()
            except Exception:
                # Shared journal/host cleanup failures cannot be hidden behind
                # a live API accepting more work. VM expiry is independently
                # enforced by the Rust pool even when this process exits.
                print("build supervisor stopped: shared state unavailable", file=sys.stderr, flush=True)
                server.shutdown()
                return
            stop.wait(1)
    thread = threading.Thread(target=work, daemon=True)
    thread.start()
    try:
        server.serve_forever(poll_interval=0.5)
    finally:
        stop.set()
        server.server_close()
        # VM deadlines remain enforced by the independent Rust build pool.


if __name__ == "__main__":
    main()
