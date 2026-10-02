"""Fixed shared-filesystem profile, with conservative zero allocated-block credit.

The operator provisions one private coordination file for every managed pool on
this volume. The Rust host activates it only with all managed roots empty and
2x byte/inode peak free. Other profiles, unmanaged writers, external collectors
and filesystem/jailer-specific overhead beyond the declared bound are explicit
availability premises. This is not a filesystem quota or a second controller.
"""
import fcntl
import json
import os
from pathlib import Path
import stat

MAXIMUM = 2**63 - 1
PROFILE_FIELDS = {"version", "coordination_file", "pools", "public_build", "filesystem_overhead_bytes", "filesystem_overhead_inodes"}
POOL_FIELDS = {"role", "namespace", "database", "node_state_dir", "chroot_base", "rootfs_budget_bytes", "max_host_slots",
               "kernel_copy_bytes", "runtime_copy_bytes", "cleanup_reserve_bytes", "host_metadata_bytes", "host_metadata_inodes", "jail_inodes_per_slot"}
PUBLIC_FIELDS = {"root", "registry_root", "workspace_bytes", "registry_bytes", "max_jobs", "max_templates", "inode_budget"}


def positive(value):
    if type(value) is not int or not 0 < value <= MAXIMUM:
        raise ValueError("explicit representable physical budget required")
    return value


def envelope(profile):
    if (type(profile) is not dict or set(profile) != PROFILE_FIELDS or type(profile["version"]) is not int or profile["version"] != 1
            or type(profile["pools"]) is not list or not 1 <= len(profile["pools"]) <= 2):
        raise ValueError("invalid fixed physical profile")
    peak, inodes = positive(profile["filesystem_overhead_bytes"]), 1
    roots, namespaces, roles = [profile["coordination_file"]], set(), []
    for pool in profile["pools"]:
        if type(pool) is not dict or set(pool) != POOL_FIELDS:
            raise ValueError("invalid physical pool")
        namespace = pool["namespace"]
        if (not isinstance(namespace, str) or len(namespace) != 16 or any(c not in "0123456789abcdef" for c in namespace)
                or namespace in namespaces):
            raise ValueError("physical pool namespace mismatch")
        namespaces.add(namespace)
        roles.append(pool["role"])
        for field in POOL_FIELDS - {"role", "namespace", "database", "node_state_dir", "chroot_base"}:
            positive(pool[field])
        if pool["jail_inodes_per_slot"] < 4 or pool["host_metadata_bytes"] > pool["cleanup_reserve_bytes"]:
            raise ValueError("physical pool metadata bound is insufficient")
        roots.extend(pool[field] for field in ("database", "node_state_dir", "chroot_base"))
        peak += pool["rootfs_budget_bytes"] + pool["max_host_slots"] * (pool["kernel_copy_bytes"] + pool["runtime_copy_bytes"]) + pool["cleanup_reserve_bytes"]
        inodes += pool["host_metadata_inodes"] + pool["max_host_slots"] * pool["jail_inodes_per_slot"] + 7
    public = profile["public_build"]
    if sorted(roles) != (["runtime"] if public is None else ["builder", "runtime"]):
        raise ValueError("unsupported physical pool roles")
    if public is not None:
        if type(public) is not dict or set(public) != PUBLIC_FIELDS:
            raise ValueError("invalid physical public build profile")
        peak += positive(public["workspace_bytes"]) + positive(public["registry_bytes"])
        inodes += positive(public["inode_budget"])
        if public["inode_budget"] < 8 + 19 * positive(public["max_jobs"]) + 5 * positive(public["max_templates"]):
            raise ValueError("physical public inode provision is insufficient")
        roots.extend((public["root"], public["registry_root"]))
    paths = []
    for value in roots:
        if not isinstance(value, str) or not value.startswith("/") or "\0" in value or any(p in ("", ".", "..") for p in value.split("/")[1:]):
            raise ValueError("absolute canonical physical profile path required")
        path = Path(value)
        if path == Path("/") or any(path == other or path in other.parents or other in path.parents for other in paths):
            raise ValueError("physical allocation roots overlap")
        paths.append(path)
    if 2 * peak > MAXIMUM or 2 * positive(profile["filesystem_overhead_inodes"]) > MAXIMUM or profile["filesystem_overhead_inodes"] < inodes:
        raise ValueError("physical envelope overflow or insufficient inode provision")
    return peak, profile["filesystem_overhead_inodes"]


def validate_configuration(runtime, builder, build, *, file_size=None):
    """Cross-check both known configs; host additionally derives its exact metadata.

    A full profile in either pool requires the same complete profile everywhere.
    No profile and no full-capacity controller preserves the previous mode.
    """
    def optional_object(pool, key):
        value = pool.get(key)
        if value is None:
            return {}
        if type(value) is not dict:
            raise ValueError("invalid optional physical configuration: " + key)
        return value
    profiles = [optional_object(pool, "bounded_host").get("physical_storage") for pool in (runtime, builder)]
    full = [optional_object(pool, "maintenance").get("capacity") for pool in (runtime, builder)]
    if all(profile is None for profile in profiles) and all(value is None for value in full):
        return None
    if (any(type(value) is not dict or not value for value in full) or profiles[0] is None
            or profiles[0] != profiles[1] or not build.get("bounded_builds")):
        raise ValueError("full physical capacity requires identical explicit profiles and bounded pools/builds")
    profile = profiles[0]
    journal = sum(positive(build["bounded_builds"][field]) for field in ("database_bytes", "wal_bytes", "recovery_reserve_bytes"))
    if build["limits"]["workspace_bytes"] < journal:
        raise ValueError("physical workspace must contain the complete public journal reserve")
    peak, inodes = envelope(profile)
    if len(profile["pools"]) != 2 or profile["public_build"] is None:
        raise ValueError("public builds require the fixed runtime/builder physical profile")
    size = file_size or (lambda path: Path(path).stat().st_size)
    for role, settings in (("runtime", runtime), ("builder", builder)):
        matches = [pool for pool in profile["pools"] if pool["namespace"] == settings["namespace"]]
        if len(matches) != 1:
            raise ValueError("physical profile does not name this pool")
        pool = matches[0]
        host = settings["bounded_host"]
        capacity = settings["maintenance"]["capacity"]
        if (pool["role"] != role or bool(settings.get("internal_build_vms")) != (role == "builder")
                or any(pool[field] != settings[field] for field in ("database", "node_state_dir", "chroot_base"))
                or pool["rootfs_budget_bytes"] != settings["max_disk_bytes_global"] - settings["cleanup_reserve_bytes"]
                or pool["cleanup_reserve_bytes"] != settings["cleanup_reserve_bytes"]
                or pool["max_host_slots"] != host["maintenance"]["max_host_slots"]
                or pool["host_metadata_bytes"] + capacity["max_online_storage_bytes"] > pool["cleanup_reserve_bytes"]):
            raise ValueError("physical profile differs from a configured pool budget")
        kernel = max([size(settings["kernel"])] + [item["kernel"]["size_bytes"] for item in settings["templates"]["templates"]])
        if settings.get("template_registry"):
            kernel = max(kernel, settings["template_registry"]["max_artifact_bytes_per_tenant"])
        if pool["kernel_copy_bytes"] < kernel or pool["runtime_copy_bytes"] < size(settings["firecracker"]):
            raise ValueError("physical profile omits a kernel/runtime copy maximum")
    public, registry = profile["public_build"], runtime["template_registry"]
    # Registry artifact quota excludes the retained manifest and owner marker.
    registry_peak = registry["max_artifact_bytes_global"] + registry["max_templates_global"] * (64 * 1024 + 87)
    # Workspace: root + lease/DB/WAL/SHM, final (directory + nine files)
    # and preparing (directory + two marker names) per retained job. Candidate
    # staging: one directory + up to five files per job. Published registry:
    # registry/inbox/artifacts + directory/kernel/rootfs/manifest/owner per row.
    inode_minimum = 8 + 19 * build["limits"]["max_jobs"] + 5 * registry["max_templates_global"]
    if (public["root"] != build["root"] or public["registry_root"] != registry["root"]
            or public["workspace_bytes"] != build["limits"]["workspace_bytes"]
            or public["max_jobs"] != build["limits"]["max_jobs"] or public["max_templates"] != registry["max_templates_global"]
            or public["registry_bytes"] < registry_peak or public["inode_budget"] < inode_minimum):
        raise ValueError("physical profile omits public workspace/registry envelope")
    return profile, peak, inodes


class PhysicalProfileLease:
    """Read-only admission lease; only the Rust host may activate fresh roots."""
    def __init__(self, profile):
        self.profile = profile
        self.peak, self.inodes = envelope(profile)
        path = Path(profile["coordination_file"])
        self.file = path.open("rb")
        try:
            fcntl.flock(self.file.fileno(), fcntl.LOCK_SH | fcntl.LOCK_NB)
            self.identity = (os.fstat(self.file.fileno()).st_dev, os.fstat(self.file.fileno()).st_ino)
            self.validate()
            # Complete bytes may follow an initializer's uncertain fsync.
            # Confirm their durability; Python never writes/repairs a prefix.
            os.fsync(self.file.fileno())
            parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                os.fsync(parent)
            finally:
                os.close(parent)
        except BaseException:
            self.file.close()
            raise

    def validate(self):
        path = Path(self.profile["coordination_file"])
        if path.resolve(strict=True) != path:
            raise ValueError("physical profile coordination path changed")
        meta, current = os.fstat(self.file.fileno()), path.lstat()
        if (not stat.S_ISREG(meta.st_mode) or stat.S_ISLNK(current.st_mode) or meta.st_nlink != 1
                or meta.st_uid != os.geteuid() or meta.st_mode & 0o077 or meta.st_size > 16 * 1024 * 1024
                or (current.st_dev, current.st_ino) != self.identity or (meta.st_dev, meta.st_ino) != self.identity):
            raise ValueError("physical profile coordination identity changed")
        self.file.seek(0)
        encoded = self.file.read(16 * 1024 * 1024 + 1)
        marker = json.loads(encoded)
        if marker != {"version": 1, "device": meta.st_dev, "inode": meta.st_ino, "profile": self.profile}:
            raise ValueError("physical profile is unactivated or changed")
        paths = [Path(pool["database"]).parent for pool in self.profile["pools"]]
        paths += [Path(pool[field]) for pool in self.profile["pools"] for field in ("node_state_dir", "chroot_base")]
        if self.profile["public_build"]:
            paths += [Path(self.profile["public_build"][field]) for field in ("root", "registry_root")]
        for path in paths:
            metadata = path.lstat()
            if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077
                    or metadata.st_dev != meta.st_dev or path.resolve(strict=True) != path):
                raise ValueError("physical profile paths must be private on the shared filesystem")
        space = os.statvfs(self.file.fileno())
        if self.inodes * space.f_frsize + len(encoded) > self.profile["filesystem_overhead_bytes"]:
            raise ValueError("physical overhead does not cover block rounding")

    def admit(self, free_bytes):
        self.validate()
        space = os.statvfs(self.file.fileno())
        return free_bytes >= self.peak and space.f_favail >= self.inodes

    def close(self):
        if not self.file.closed:
            fcntl.flock(self.file.fileno(), fcntl.LOCK_UN)
            self.file.close()
