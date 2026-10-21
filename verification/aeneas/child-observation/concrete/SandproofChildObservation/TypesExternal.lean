import SandboxModel.Boundaries.ChildObservation.Interface

/-! These are logical representations of the concrete foreign arguments.
The SystemIo representation is a per-invocation observation context, not a
claim that Rust's empty marker stores history or the global OS. -/
namespace SandproofChildObservationRust

abbrev core.io.error.Error := SandboxModel.Boundaries.ChildObservation.Interface.IoError
abbrev std.process.Child := SandboxModel.Boundaries.ChildObservation.Interface.Child
abbrev std.process.ExitStatus := Int
abbrev std.time.Instant := Nat
abbrev child_process.observation_io.SystemIo := SandboxModel.Boundaries.ChildObservation.Interface.Io
abbrev child_process.reaper.Slot := SandboxModel.Boundaries.ChildObservation.Interface.Slot

end SandproofChildObservationRust
