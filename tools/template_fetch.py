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
    from .template_inputs import InvalidTemplate, bounded_json, extract_archive, inspect_oci, blob, sha256
else:
    from template_inputs import InvalidTemplate, bounded_json, extract_archive, inspect_oci, blob, sha256

REGCTL_VERSION = "v0.11.6"
REGCTL_SHA256 = "8e0e62a497fcdb8048d18aa927a139613176ba0531f412bc541044e28f9856bd"
DOCKER_MANIFEST = "application/vnd.docker.distribution.manifest.v2+json"
OCI_MANIFEST = "application/vnd.oci.image.manifest.v1+json"
OCI_INDEX = "application/vnd.oci.image.index.v1+json"
DOCKER_INDEX = "application/vnd.docker.distribution.manifest.list.v2+json"


def reference(value):
    # Normalize Docker's public shorthand without interpreting any user text
    # as code. Address isolation remains the host network policy.
    if not isinstance(value, str) or len(value) > 512:
        raise InvalidTemplate("invalid public image reference")
    match = re.fullmatch(r"([a-z0-9][a-z0-9./_-]*)(@sha256:[0-9a-f]{64}|:[A-Za-z0-9_][A-Za-z0-9_.-]{0,127})?", value)
    if match is None:
        raise InvalidTemplate("invalid public image reference")
    name, suffix = match.groups()
    parts = name.split("/", 1)
    if len(parts) == 2 and ("." in parts[0] or parts[0] == "localhost"):
        host, repo = parts
    else:
        host, repo = "docker.io", name if "/" in name else "library/" + name
    if not re.fullmatch(r"[a-z0-9]+(?:[._-]+[a-z0-9]+)*(?:/[a-z0-9]+(?:[._-]+[a-z0-9]+)*)*", repo):
        raise InvalidTemplate("invalid public image repository")
    labels = host.split(".")
    if (len(host) > 253 or len(labels) < 2 or not labels[-1].isalpha()
            or any(not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", label) for label in labels)
            or labels[-1] in ("localhost", "local", "internal", "invalid", "test", "onion", "lan", "home")):
        raise InvalidTemplate("public DNS registry required")
    return host + "/" + repo, suffix[1:] if suffix else "latest"


def select_manifest(document, digest):
    if not isinstance(document, dict) or document.get("schemaVersion") != 2:
        raise InvalidTemplate("unsupported public manifest")
    if document.get("mediaType") in (OCI_MANIFEST, DOCKER_MANIFEST):
        return digest
    if document.get("mediaType") not in (OCI_INDEX, DOCKER_INDEX):
        raise InvalidTemplate("public image must provide an OCI or Docker schema2 manifest")
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
            if (item.get("mediaType") not in (OCI_MANIFEST, DOCKER_MANIFEST)
                    or not isinstance(item.get("digest"), str)
                    or not re.fullmatch(r"sha256:[0-9a-f]{64}", item["digest"])
                    or type(item.get("size")) is not int or not 0 < item["size"] <= 1024 * 1024):
                raise InvalidTemplate("invalid public image descriptor")
            selected.append(item["digest"])
    if len(selected) != 1:
        raise InvalidTemplate("expected one linux/amd64 image")
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
    repository, identifier = reference(document["image"])
    canonical = repository + ("@" if identifier.startswith("sha256:") else ":") + identifier
    if sha256(tool) != REGCTL_SHA256:
        raise InvalidTemplate("fixed regctl fingerprint required")
    manifest_path = work / "source-manifest.json"
    metadata_bytes = capture([str(tool), "manifest", "get", canonical, "--format", "raw-body"],
                             manifest_path, min(request["max_input_bytes"], 1024 * 1024), commands)
    digest = "sha256:" + sha256(manifest_path)
    if identifier.startswith("sha256:") and digest != identifier:
        raise InvalidTemplate("public source manifest digest mismatch")
    # A tag is resolved once. All subsequent layer/config acquisition uses
    # the selected digest, so a tag update cannot mix two image revisions.
    selected = select_manifest(bounded_json(manifest_path), digest)
    archive = work / "download.tar"
    archive_bytes = capture([str(tool), "image", "export", repository + "@" + selected],
                            archive, request["max_input_bytes"] - metadata_bytes, commands)
    extract_archive(archive, layout, max_bytes=request["max_input_bytes"], max_entries=request["max_entries"])
    index = bounded_json(layout / "index.json")
    items = index.get("manifests") if isinstance(index, dict) else None
    if (not isinstance(items, list) or len(items) != 1 or not isinstance(items[0], dict)
            or items[0].get("digest") != selected or items[0].get("mediaType") not in (OCI_MANIFEST, DOCKER_MANIFEST)):
        raise InvalidTemplate("downloaded manifest does not match selected digest")
    converted = None
    if items[0]["mediaType"] == DOCKER_MANIFEST:
        # Validate the original bytes before the trusted converter sees them.
        # Conversion is local-only and must preserve config/layer bytes exactly.
        limits = {"max_bytes": request["max_expanded_bytes"], "max_entries": request["max_entries"]}
        inspect_oci(layout, **limits, allow_docker=True)
        original = payload_identity(layout, request["max_input_bytes"])
        raw = work / "source-oci"
        layout.rename(raw)
        archive.unlink()  # two layout copies replace tar+layout in disk budget
        commands.run([str(tool), "image", "mod", "ocidir://" + str(raw) + "@" + selected,
                      "--to-oci", "--create", "ocidir://" + str(layout) + ":candidate"])
        normalized = inspect_oci(layout, **limits)
        if payload_identity(layout, request["max_input_bytes"]) != original:
            raise InvalidTemplate("OCI conversion changed image config or layers")
        converted = normalized["image_digest"]
    # inspect_oci rehashes every manifest/config/layer and checks diff_ids,
    # architecture, expanded bytes and entries before any umoci unpacking.
    result = {"reference": document["image"], "source_digest": digest,
            "selected_digest": selected, "platform": "linux/amd64",
            "download_bytes": metadata_bytes + archive_bytes, "regctl_version": REGCTL_VERSION,
            "regctl_sha256": REGCTL_SHA256}
    if converted is not None:
        result["converted_digest"] = converted
        result["conversion"] = "docker-schema2-to-oci"
    return result


def payload_identity(layout, maximum):
    """Compare ordered immutable blobs after inspect_oci has verified them."""
    descriptor = bounded_json(layout / "index.json")["manifests"][0]
    manifest = bounded_json(blob(layout, descriptor, maximum))
    return [(item["digest"], item["size"]) for item in [manifest["config"], *manifest["layers"]]]
