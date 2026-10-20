import SandproofChildObservation.Types
import SandboxModel.Boundaries.ChildObservation.Library

open Aeneas Aeneas.Std
namespace SandproofChildObservationRust

/-- The frozen generator passes a dictionary; the stock model takes its
partial comparison function. This local ABI projection preserves that body. -/
@[trait_default]
def core.cmp.PartialOrd.ge.default {T R : Type}
    (dictionary : Aeneas.Std.core.cmp.PartialOrd T R) (left : T) (right : R) : Result Bool :=
  Aeneas.Std.core.cmp.PartialOrd.ge_body dictionary.partial_cmp left right


def std.time.Instant.Insts.CoreCloneClone.clone (instant : std.time.Instant) :
    Result std.time.Instant := .ok instant

def std.time.Instant.Insts.CoreCmpPartialEqInstant.eq :=
  SandboxModel.Boundaries.ChildObservation.Library.natEq.eq

def std.time.Instant.Insts.CoreCmpPartialOrdInstant.partial_cmp :=
  SandboxModel.Boundaries.ChildObservation.Library.natCmp

namespace child_process.observation_io.SystemIo.Insts.Sandbox_runtimeChild_processObservation_coreOperationsChildSlotExitStatusErrorInstant

def try_wait := SandboxModel.Boundaries.ChildObservation.Library.poll
def kill := SandboxModel.Boundaries.ChildObservation.Library.kill
def close_pipes := SandboxModel.Boundaries.ChildObservation.Library.close
def now := SandboxModel.Boundaries.ChildObservation.Library.now
def add_grace := SandboxModel.Boundaries.ChildObservation.Library.grace
def pause := SandboxModel.Boundaries.ChildObservation.Library.pause
def retain := SandboxModel.Boundaries.ChildObservation.Library.retain
def unknown_exit := SandboxModel.Boundaries.ChildObservation.Library.error

end child_process.observation_io.SystemIo.Insts.Sandbox_runtimeChild_processObservation_coreOperationsChildSlotExitStatusErrorInstant

/-- Only a fresh local observation context; not a reset of external state.
The original Child supplies a representation of this invocation's responses. -/
def child_process.observation_io.system : Result child_process.observation_io.SystemIo :=
  .ok {}

end SandproofChildObservationRust
