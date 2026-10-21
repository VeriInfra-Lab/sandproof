import Aeneas

/-! A3 projection of one valid, already obtained Linux Metadata value.
This is not a live filesystem, a path, a descriptor, or an acquisition proof. -/
namespace SandboxModel.Boundaries.ProtectedDirectory
open Aeneas Aeneas.Std

structure Snapshot where
  uid : U32
  mode : U32
  isDirectory : Bool

end SandboxModel.Boundaries.ProtectedDirectory

@[rust_type "std::fs::Metadata"]
abbrev std.fs.Metadata := SandboxModel.Boundaries.ProtectedDirectory.Snapshot

open Aeneas Aeneas.Std

@[rust_fun "std::fs::{std::fs::Metadata}::is_dir"]
def std.fs.Metadata.is_dir (metadata : std.fs.Metadata) : Result Bool :=
  .ok metadata.isDirectory

@[rust_fun "std::os::unix::fs::{std::os::unix::fs::MetadataExt<std::fs::Metadata>}::mode"]
def std.fs.Metadata.Insts.StdOsUnixFsMetadataExt.mode
    (metadata : std.fs.Metadata) : Result U32 := .ok metadata.mode

@[rust_fun "std::os::unix::fs::{std::os::unix::fs::MetadataExt<std::fs::Metadata>}::uid"]
def std.fs.Metadata.Insts.StdOsUnixFsMetadataExt.uid
    (metadata : std.fs.Metadata) : Result U32 := .ok metadata.uid
