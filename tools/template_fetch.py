"""Guest-only public OCI acquisition through a fixed regctl executable.

No registry protocol, credentials, Dockerfile evaluation, or host unpacking is
implemented here. The caller owns the disposable VM and its public IPv4 policy.
"""
import os
import re
import selectors
import signal
import subprocess
import time

if __package__:
    from .template_inputs import InvalidTemplate, bounded_json, extract_archive, sha256
else:
    from template_inputs import InvalidTemplate, bounded_json, extract_archive, sha256

REGCTL_VERSION = "v0.11.6"
REGCTL_SHA256 = "8e0e62a497fcdb8048d18aa927a139613176ba0531f412bc541044e28f9856bd"
OCI_MANIFEST = "application/vnd.oci.image.manifest.v1+json"
OCI_INDEX = "application/vnd.oci.image.index.v1+json"
DOCKER_INDEX = "application/vnd.docker.distribution.manifest.list.v2+json"


def reference(value):
    # Explicit DNS registry, no shorthand, tags, credentials, ports, scheme,
    # flags or shell syntax. Actual address isolation is the host network policy.
    if not isinstance(value, str) or len(value) > 512:
        raise InvalidTemplate("invalid public image reference")
    match = re.fullmatch(r"([a-z0-9][a-z0-9.-]*)/([a-z0-9]+(?:[._-]+[a-z0-9]+)*(?:/[a-z0-9]+(?:[._-]+[a-z0-9]+)*)*)@sha256:([0-9a-f]{64})", value)
    if match is None:
        raise InvalidTemplate("public image requires registry/repository@sha256 digest")
    host, repo, digest = match.groups()
    labels = host.split(".")
    if (len(host) > 253 or len(labels) < 2 or not labels[-1].isalpha()
            or any(not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", label) for label in labels)
            or labels[-1] in ("localhost", "local", "internal", "invalid", "test", "onion", "lan", "home")):
        raise InvalidTemplate("public DNS registry required")
    return host + "/" + repo, "sha256:" + digest


def select_manifest(document, digest):
    if not isinstance(document, dict) or document.get("schemaVersion") != 2:
        raise InvalidTemplate("unsupported public manifest")
    if document.get("mediaType") == OCI_MANIFEST:
        return digest
    if document.get("mediaType") not in (OCI_INDEX, DOCKER_INDEX):
        raise InvalidTemplate("public image must provide an OCI manifest")
    manifests = document.get("manifests")
    if not isinstance(manifests, list) or not 1 <= len(manifests) <= 128:
        raise InvalidTemplate("public image index limit")
    selected = []
    for item in manifests:
        if not isinstance(item, dict) or not isinstance(item.get("platform"), dict):
            raise InvalidTemplate("invalid public image platform descriptor")
        platform = item["platform"]
        if (platform.get("os"), platform.get("architecture")) == ("linux", "amd64"):
            if platform.get("variant", "") not in ("", "v1") or platform.get("os.version") or platform.get("os.features"):
                raise InvalidTemplate("unsupported platform variant")
            if (item.get("mediaType") != OCI_MANIFEST
                    or not isinstance(item.get("digest"), str)
                    or not re.fullmatch(r"sha256:[0-9a-f]{64}", item["digest"])
                    or type(item.get("size")) is not int or not 0 < item["size"] <= 1024 * 1024):
                raise InvalidTemplate("invalid public image descriptor")
            selected.append(item["digest"])
    if len(selected) != 1:
        raise InvalidTemplate("expected one linux/amd64 OCI image")
    return selected[0]


def stop(child):
    # regctl is a fixed, trusted executable. Only signal its private group
    # while its leader is unreaped, so a recycled process ID cannot be targeted.
    if child.poll() is None:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    child.wait(timeout=5)


def capture(args, output, limit, commands):
    """Separate bounded binary stdout from the shared bounded diagnostic log."""
    if time.monotonic() >= commands.deadline:
        raise TimeoutError("public image deadline")
    child = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, start_new_session=True,
        env={"PATH": "/usr/local/bin:/usr/sbin:/usr/bin:/bin", "LC_ALL": "C",
             "HOME": "/var/empty", "DOCKER_CONFIG": "/var/empty", "REGCTL_CONFIG": "/var/empty/regctl.json"})
    total = 0
    try:
        with output.open("xb") as target, selectors.DefaultSelector() as ready:
            ready.register(child.stdout, selectors.EVENT_READ, "output")
            ready.register(child.stderr, selectors.EVENT_READ, "log")
            while ready.get_map():
                remaining = commands.deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("public image deadline")
                for key, _ in ready.select(timeout=min(0.2, remaining)):
                    data = os.read(key.fd, 65536)
                    if not data:
                        ready.unregister(key.fileobj)
                    elif key.data == "output":
                        if len(data) > limit - total:
                            raise InvalidTemplate("public image download byte limit")
                        target.write(data)
                        total += len(data)
                    else:
                        accepted = commands.budget.take(len(data))
                        commands.log.write(data[:accepted]); commands.log.flush()
                        if accepted < len(data):
                            raise InvalidTemplate("public image log limit")
            try:
                result = child.wait(timeout=max(0, commands.deadline - time.monotonic()))
            except subprocess.TimeoutExpired as error:
                raise TimeoutError("public image deadline") from error
            if result != 0:
                raise InvalidTemplate("public image fetch failed")
        return total
    finally:
        stop(child)
        child.stdout.close(); child.stderr.close()


def fetch(source, work, layout, request, tool, commands):
    document = bounded_json(source, 4096)
    if not isinstance(document, dict) or set(document) != {"image"}:
        raise InvalidTemplate("expected a public image reference document")
    repository, digest = reference(document["image"])
    if sha256(tool) != REGCTL_SHA256:
        raise InvalidTemplate("fixed regctl fingerprint required")
    manifest_path = work / "source-manifest.json"
    metadata_bytes = capture([str(tool), "manifest", "get", document["image"], "--format", "raw-body"],
                             manifest_path, min(request["max_input_bytes"], 1024 * 1024), commands)
    if "sha256:" + sha256(manifest_path) != digest:
        raise InvalidTemplate("public source manifest digest mismatch")
    selected = select_manifest(bounded_json(manifest_path), digest)
    archive = work / "download.tar"
    archive_bytes = capture([str(tool), "image", "export", repository + "@" + selected],
                            archive, request["max_input_bytes"] - metadata_bytes, commands)
    extract_archive(archive, layout, max_bytes=request["max_input_bytes"], max_entries=request["max_entries"])
    index = bounded_json(layout / "index.json")
    items = index.get("manifests") if isinstance(index, dict) else None
    if (not isinstance(items, list) or len(items) != 1 or not isinstance(items[0], dict)
            or items[0].get("digest") != selected or items[0].get("mediaType") != OCI_MANIFEST):
        raise InvalidTemplate("downloaded manifest does not match selected digest")
    # inspect_oci rehashes every manifest/config/layer and checks diff_ids,
    # architecture, expanded bytes and entries before any umoci unpacking.
    return {"reference": document["image"], "source_digest": digest,
            "selected_digest": selected, "platform": "linux/amd64",
            "download_bytes": metadata_bytes + archive_bytes, "regctl_version": REGCTL_VERSION,
            "regctl_sha256": REGCTL_SHA256}
