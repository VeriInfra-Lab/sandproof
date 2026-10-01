"""Bounded template inputs. No Dockerfile interpretation or host execution."""

import gzip
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat
import tarfile


class InvalidTemplate(ValueError):
    pass


def platform_dns_servers(value):
    """Trusted public IPv4 unicast resolver addresses; never a query filter."""
    import ipaddress
    if not isinstance(value, list) or not 1 <= len(value) <= 2:
        raise InvalidTemplate("one or two public IPv4 DNS strings required")
    for item in value:
        if not isinstance(item, str):
            raise InvalidTemplate("public IPv4 DNS addresses must be strings")
        try:
            address = ipaddress.IPv4Address(item)
        except ValueError as error:
            raise InvalidTemplate("invalid public IPv4 DNS address") from error
        if not address.is_global or address.is_multicast or address.is_reserved:
            raise InvalidTemplate("public unicast IPv4 DNS address required")
    return value


def image_path(name, *, allow_backslash=False):
    if (not isinstance(name, str) or not name or "\x00" in name
            or (not allow_backslash and "\\" in name)):
        raise InvalidTemplate("invalid archive path")
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts:
        raise InvalidTemplate("archive path escapes root")
    return path


def relative_link(name, target):
    if not target or target.startswith("/") or "\\" in target or "\x00" in target:
        raise InvalidTemplate("context link must be relative")
    parts = list(image_path(name).parent.parts)
    for part in PurePosixPath(target).parts:
        if part == "..":
            if not parts:
                raise InvalidTemplate("context link escapes root")
            parts.pop()
        elif part != ".":
            parts.append(part)


def check_members(archive, *, max_bytes, max_entries, context):
    """Validate without extracting; umoci handles actual OCI layer semantics.

    Absolute image symlinks are normal (e.g. /bin); these are allowed only in
    OCI layers, never followed here. Linux layer names/links retain literal
    backslashes (e.g. systemd escaped unit names); never decode or replace them.
    Contexts reject backslashes, symlink ancestors, hard
    links, devices, FIFOs, duplicate paths and sparse payloads.
    """
    members, names, links, total = [], set(), set(), 0
    for entry in archive:
        path = image_path(entry.name, allow_backslash=not context)
        if len(members) >= max_entries or entry.size < 0 or entry.sparse:
            raise InvalidTemplate("archive entry limit or sparse payload")
        total += entry.size
        if total > max_bytes:
            raise InvalidTemplate("archive expanded byte limit")
        if not (entry.isfile() or entry.isdir() or entry.issym() or (not context and entry.islnk())):
            raise InvalidTemplate("unsupported archive file type")
        if entry.islnk():
            image_path(entry.linkname, allow_backslash=True)
        if entry.issym():
            if context:
                relative_link(entry.name, entry.linkname)
            elif not entry.linkname or "\x00" in entry.linkname:
                raise InvalidTemplate("invalid image symlink")
            links.add(path)
        if context and path in names:
            raise InvalidTemplate("duplicate context path")
        names.add(path)
        members.append(entry)
    if context:
        for path in names:
            if any(parent in links for parent in path.parents):
                raise InvalidTemplate("context entry below symlink")
    return members


def extract_archive(source, destination, *, max_bytes, max_entries, dockerfile=False):
    destination.mkdir(mode=0o700)  # never reuse a staging tree
    with tarfile.open(source, "r:*") as archive:
        entries = check_members(archive, max_bytes=max_bytes, max_entries=max_entries, context=True)
        # No extractall: regular bytes first, symlinks last. No user ownership,
        # setuid bits, device files or host path interpretation is preserved.
        for entry in entries:
            target = destination / image_path(entry.name)
            if entry.issym():
                continue
            if target == destination:
                if not entry.isdir():
                    raise InvalidTemplate("context root must be directory")
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            if entry.isdir():
                target.mkdir(exist_ok=True)
            else:
                with archive.extractfile(entry) as incoming, target.open("xb") as outgoing:
                    remaining = entry.size
                    while remaining:
                        block = incoming.read(min(remaining, 1024 * 1024))
                        if not block:
                            raise InvalidTemplate("truncated context")
                        outgoing.write(block)
                        remaining -= len(block)
                target.chmod(entry.mode & 0o777)
        for entry in entries:
            if entry.issym():
                target = destination / image_path(entry.name)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.symlink_to(entry.linkname)
    if dockerfile:
        definition = destination / "Dockerfile"
        if not definition.is_file() or definition.is_symlink():
            raise InvalidTemplate("context must contain a regular Dockerfile")


def extract_context(source, destination, **limits):
    extract_archive(source, destination, dockerfile=True, **limits)


def sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def bounded_json(path, limit=1024 * 1024):
    if not stat.S_ISREG(path.lstat().st_mode) or path.stat().st_size > limit:
        raise InvalidTemplate("JSON type or size limit")
    return json.loads(path.read_bytes())


def blob(layout, descriptor, max_bytes):
    digest = descriptor.get("digest", "")
    size = descriptor.get("size")
    if (not re.fullmatch(r"sha256:[0-9a-f]{64}", digest) or type(size) is not int
            or not 0 < size <= max_bytes):
        raise InvalidTemplate("invalid OCI descriptor")
    path = layout / "blobs/sha256" / digest[7:]
    if (not stat.S_ISREG(path.lstat().st_mode) or path.stat().st_size != size
            or sha256(path) != digest[7:]):
        raise InvalidTemplate("OCI blob fingerprint mismatch")
    return path


class LayerReader:
    """Hash and bound every decompressed byte as the tar parser consumes it."""

    def __init__(self, stream, maximum):
        self.stream, self.maximum = stream, maximum
        self.digest, self.expanded = hashlib.sha256(), 0

    def read(self, size=-1):
        size = 1024 * 1024 if size < 0 else min(size, 1024 * 1024)
        block = self.stream.read(min(size, self.maximum - self.expanded + 1))
        self.expanded += len(block)
        if self.expanded > self.maximum:
            raise InvalidTemplate("OCI expanded byte limit")
        self.digest.update(block)
        return block


def inspect_oci(layout, *, max_bytes, max_entries, allow_docker=False):
    """Only one linux/amd64 manifest; no silently chosen multiarch/attestation."""
    if bounded_json(layout / "oci-layout") != {"imageLayoutVersion": "1.0.0"}:
        raise InvalidTemplate("unsupported OCI layout")
    index = bounded_json(layout / "index.json")
    manifests = index.get("manifests", [])
    if index.get("schemaVersion") != 2 or len(manifests) != 1:
        raise InvalidTemplate("expected one OCI image, no index selection")
    descriptor = manifests[0]
    docker = descriptor.get("mediaType") == "application/vnd.docker.distribution.manifest.v2+json"
    if descriptor.get("mediaType") != "application/vnd.oci.image.manifest.v1+json" and not (allow_docker and docker):
        raise InvalidTemplate("unsupported OCI manifest")
    config_media = "application/vnd.docker.container.image.v1+json" if docker else "application/vnd.oci.image.config.v1+json"
    layer_media = ("application/vnd.docker.image.rootfs.diff.tar.gzip",) if docker else (
        "application/vnd.oci.image.layer.v1.tar", "application/vnd.oci.image.layer.v1.tar+gzip")
    manifest = bounded_json(blob(layout, descriptor, max_bytes))
    if (manifest.get("schemaVersion") != 2
            or manifest.get("config", {}).get("mediaType") != config_media):
        raise InvalidTemplate("unsupported OCI image config")
    config = bounded_json(blob(layout, manifest["config"], max_bytes))
    if (config.get("os"), config.get("architecture")) != ("linux", "amd64"):
        raise InvalidTemplate("only linux/amd64 supported")
    layers = manifest.get("layers", [])
    diff_ids = config.get("rootfs", {}).get("diff_ids", [])
    if (config.get("rootfs", {}).get("type") != "layers"
            or not layers or len(layers) > 128 or len(diff_ids) != len(layers)):
        raise InvalidTemplate("invalid image layers")
    remaining_bytes, remaining_entries = max_bytes, max_entries
    for layer, expected_diff in zip(layers, diff_ids):
        path = blob(layout, layer, max_bytes)
        media = layer.get("mediaType")
        if media not in layer_media or layer.get("urls"):
            raise InvalidTemplate("only tar/gzip OCI layers supported")
        opener = gzip.open if media.endswith(("+gzip", ".gzip")) else open
        # Parse headers and verify diff_id through the same forward-only
        # decompressor. Drain after tar EOF: padding and trailing gzip members
        # still count toward the byte limit, digest and gzip integrity checks.
        with opener(path, "rb") as source:
            stream = LayerReader(source, remaining_bytes)
            with tarfile.open(fileobj=stream, mode="r|", bufsize=1024 * 1024) as archive:
                entries = check_members(archive, max_bytes=remaining_bytes,
                                        max_entries=remaining_entries, context=False)
            while stream.read(1024 * 1024):
                pass
        if expected_diff != "sha256:" + stream.digest.hexdigest():
            raise InvalidTemplate("OCI diff_id mismatch")
        remaining_bytes -= stream.expanded
        remaining_entries -= len(entries)
    return {"image_digest": descriptor["digest"], "config_digest": manifest["config"]["digest"],
            "container_config": config.get("config", {})}


def startup_config(config):
    """Retain OCI argv and defaults; normalization happens inside the build VM."""
    result = {}
    for field, fallback in (("Env", []), ("WorkingDir", "/"), ("User", "root"),
                            ("Entrypoint", []), ("Cmd", [])):
        value = config.get(field)
        if value is None or value == "":
            value = fallback
        if isinstance(fallback, list):
            if not isinstance(value, list) or len(value) > 256 or any(
                    not isinstance(item, str) or "\x00" in item or len(item.encode()) > 8192 for item in value):
                raise InvalidTemplate("invalid container " + field)
        elif not isinstance(value, str) or "\x00" in value or len(value.encode()) > 4096:
            raise InvalidTemplate("invalid container " + field)
        result[field] = value
    image_path(result["WorkingDir"].lstrip("/")) if result["WorkingDir"] != "/" else None
    if not result["WorkingDir"].startswith("/"):
        raise InvalidTemplate("WORKDIR must be absolute")
    if config.get("Volumes"):
        raise InvalidTemplate("container volumes not supported")
    for env in result["Env"]:
        key, sep, _ = env.partition("=")
        if not sep or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            raise InvalidTemplate("invalid container ENV")
    return result
