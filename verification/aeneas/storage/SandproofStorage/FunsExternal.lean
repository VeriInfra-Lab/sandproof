import SandproofStorage.Types
import SandproofStorage.Library
import SandproofFoundations.FunsExternal

/- External API value models for the registered plain-string/byte domain.
The connection schedule describes possible adapter replies. The request log
records adapter invocation parameters, not transmission or durable acceptance.
Physical effects on errors remain unconstrained by this value model. -/
open Aeneas Aeneas.Std
namespace SandproofStorageRust

def toStr (text : String)
    (bounded : text.toByteArray.size ≤ U32.max := by
      rw [U32.max_eq]
      set_option maxRecDepth 8192 in decide) : Str :=
  SandproofFoundationsRust.toStr text bounded

def Shared0T.Insts.CoreFmtDisplay.fmt {T : Type}
    (display : core.fmt.Display T) (value : T) (formatter : core.fmt.Formatter) :=
  SandproofFoundationsRust.Shared0T.Insts.CoreFmtDisplay.fmt display value formatter

def Str.Insts.CoreFmtDisplay.fmt (value : Str) (formatter : core.fmt.Formatter) :=
  SandproofFoundationsRust.core.fmt.Formatter.write_str formatter value

def core.fmt.num.imp.DisplayU64.fmt (value : U64) (formatter : core.fmt.Formatter) :=
  SandproofFoundationsRust.core.fmt.num.imp.DisplayU64.fmt value formatter

def core.fmt.rt.Argument.new_display {T : Type} (display : core.fmt.Display T) (value : T) :=
  SandproofFoundationsRust.core.fmt.rt.Argument.new_display display value

def core.fmt.Arguments.new {N M : Usize} (bytes : Aeneas.Std.Array U8 N)
    (arguments : Aeneas.Std.Array core.fmt.rt.Argument M) :=
  SandproofFoundationsRust.core.fmt.Arguments.new bytes arguments

def core.hint.must_use {T : Type} (value : T) : Result T := .ok value

def core.option.Option.Insts.CoreCmpPartialEqOption.eq {T : Type}
    (compare : core.cmp.PartialEq T T) (left right : Option T) : Result Bool :=
  match left, right with
  | none, none => .ok true
  | some l, some r => compare.eq l r
  | _, _ => .ok false

def core.result.Result.is_err {T E : Type} (value : core.result.Result T E) : Result Bool :=
  match value with
  | .Ok _ => .ok false
  | .Err _ => .ok true

def core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next :=
  LibraryBytes.nextLine

def core.str.Str.lines (value : Str) : Result core.str.iter.Lines := .ok ⟨value⟩

def Str.Insts.CoreCmpPartialOrdStr.partial_cmp (left right : Str) : Result (Option Ordering) :=
  .ok (some (LibraryBytes.compare left.val right.val))

def Str.Insts.CoreCmpPartialEqStr.eq (left right : Str) : Result Bool :=
  SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq left right

def Str.Insts.CoreCmpOrd.cmp (left right : Str) : Result Ordering :=
  .ok (LibraryBytes.compare left.val right.val)

def lookupBorrowed {K V Q : Type} (getKey : K → Result Q)
    (compareKey : Q → Q → Result Ordering) (key : Q) : List (K × V) → Result (Option V)
  | [] => .ok none
  | entry :: rest => do
    let actual ← getKey entry.1
    let order ← compareKey actual key
    if order = .eq then .ok (some entry.2) else lookupBorrowed getKey compareKey key rest

/- Only the frozen String/str dictionaries are registered for this operation.
Their comparisons are pure; valid map entries have unique keys. An arbitrary
stateful comparison or user Borrow implementation is a new interface use. -/
def alloc.collections.btree.map.BTreeMap.get {K V A Q : Type}
    (_clone : core.clone.Clone A) (borrow : core.borrow.Borrow K Q)
    (_keyOrder : core.cmp.Ord K) (queryOrder : core.cmp.Ord Q)
    (value : alloc.collections.btree.map.BTreeMap K V A) (key : Q) : Result (Option V) :=
  lookupBorrowed borrow.borrow queryOrder.cmp key value.entries

def alloc.fmt.format (arguments : core.fmt.Arguments) : Result String :=
  if arguments.bytecode.map (·.val) = [20] ++
      (SandproofFoundationsRust.IdentifierLibrary.stringBytes "sandproof:authority:").map (·.val) ++ [192, 0] then
    match arguments.arguments with
    | [argument] => do
      let (result, formatter) ← argument.render ⟨"sandproof:authority:"⟩
      match result with
      | .Ok _ => .ok formatter.output
      | .Err _ => .fail .panic
    | _ => .fail .panic
  else .fail .panic

def alloc.string.String.Insts.CoreBorrowBorrowStr.borrow (value : String) : Result Str :=
  SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref value

def alloc.string.String.Insts.CoreCmpPartialEqString.eq (left right : String) : Result Bool :=
  SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq left right

def alloc.string.String.Insts.CoreCmpPartialOrdString.partial_cmp (left right : String) :
    Result (Option Ordering) :=
  .ok (some (LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes left)
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes right)))

def alloc.string.String.Insts.CoreCmpOrd.cmp (left right : String) : Result Ordering :=
  .ok (LibraryBytes.compare (SandproofFoundationsRust.IdentifierLibrary.stringBytes left)
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes right))

def alloc.string.String.new : Result String := .ok ""

def alloc.string.String.as_str (value : String) : Result Str :=
  SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref value

def alloc.string.String.Insts.CoreCloneClone.clone (value : String) : Result String := .ok value

def alloc.string.String.Insts.CoreOpsDerefDerefStr.deref (value : String) : Result Str :=
  SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref value

def alloc.string.ToString.Blanket.to_string {T : Type}
    (display : core.fmt.Display T) (value : T) : Result String :=
  SandproofFoundationsRust.alloc.string.ToString.Blanket.to_string display value

def alloc.vec.Vec.as_slice {T : Type} (_A : Type) (value : alloc.vec.Vec T) : Result (Slice T) :=
  .ok (Aeneas.Std.alloc.vec.Vec.deref value)

def invoke (connection : Option redis.connection.Connection) (request : LibraryRequest) :
    LibraryReply × Option redis.connection.Connection :=
  match connection with
  | none => (.refusedBeforeCall, none)
  | some current =>
    match current.replies with
    | [] => (.deliveryUnknown, none)
    | reply :: rest =>
      (reply, some { current with replies := rest, requests := current.requests ++ [request] })

def readReply {T : Type} (project : LibraryReply → Option T)
    (reply : LibraryReply) (connection : Option redis.connection.Connection) :
    Result (core.result.Result T sandproof_center_store.StoreError ×
      Option redis.connection.Connection) :=
  match reply with
  | .refusedBeforeCall => .ok (.Err .Unavailable, connection)
  | .deliveryUnknown => .ok (.Err .Unavailable, none)
  | .panicked => .fail .panic
  | .nonReturning => .div
  | other =>
    match project other with
    | none => .ok (.Err .Unavailable, none)
    | some value => .ok (.Ok value, connection)

def evalRequest (input : store.storage_io.Cas) (deadline : std.time.Instant) : LibraryRequest :=
  let bytes := SandproofFoundationsRust.IdentifierLibrary.stringBytes
  ⟨"EVAL", [input.script.val, bytes "1", bytes input.key, input.deployment.val,
    bytes (toString input.format.val), bytes input.prior, bytes (toString input.revision.val),
    input.payload.val, bytes (toString input.limit.val), input.expected.val], deadline.tick⟩

def store.storage_io.eval (connection : Option redis.connection.Connection)
    (input : store.storage_io.Cas) (deadline : std.time.Instant) :
    Result (core.result.Result I64 sandproof_center_store.StoreError × Option redis.connection.Connection) :=
  let (reply, next) := invoke connection (evalRequest input deadline)
  readReply (fun | .number value => some value | _ => none) reply next

def store.storage_io.info (connection : Option redis.connection.Connection)
    (sectionName : Str) (deadline : std.time.Instant) (_phase : store.diagnostics.Phase) :
    Result (core.result.Result String sandproof_center_store.StoreError × Option redis.connection.Connection) :=
  let (reply, next) := invoke connection ⟨"INFO", [sectionName.val], deadline.tick⟩
  readReply (fun | .text value => some value | _ => none) reply next

def store.storage_io.settings (connection : Option redis.connection.Connection)
    (keys : Slice Str) (deadline : std.time.Instant) (_phase : store.diagnostics.Phase) :
    Result (core.result.Result (alloc.collections.btree.map.BTreeMap String String Global)
      sandproof_center_store.StoreError × Option redis.connection.Connection) :=
  let args := SandproofFoundationsRust.IdentifierLibrary.stringBytes "GET" :: keys.val.map (·.val)
  let (reply, next) := invoke connection ⟨"CONFIG", args, deadline.tick⟩
  readReply (fun | .settings entries => some ⟨entries⟩ | _ => none) reply next

def store.storage_io.trim_cr (value : Str) : Result Str := .ok (LibraryBytes.trimCR value)

def store.storage_io.split_colon (value : Str) : Result (Option (Str × Str)) :=
  .ok (LibraryBytes.splitColon value)

-- The new constant is the original standard-library zero duration.
def core.time.Duration.ZERO : Result core.time.Duration := .ok ⟨0, 0⟩

def diagnosticPhase (phase : store.diagnostics.Phase) : Nat :=
  match phase with
  | .ServerHealth => 0
  | .SettingsHealth => 1
  | .PersistenceHealth => 2
  | .Read => 3
  | .Cas => 4

-- Only the two failure values emitted by the transparent health path are
-- admitted here. Other diagnostics stay inside the existing query adapter.
def diagnosticFailure (failure : store.diagnostics.Failure) : Option Nat :=
  match failure with
  | .ConfigurationMismatch => some 7
  | .UnhealthyPersistence => some 8
  | _ => none

def store.storage_io.emit (output : store.storage_io.Diagnostics)
    (phase : store.diagnostics.Phase) (failure : store.diagnostics.Failure)
    (elapsed budget : core.time.Duration) : Result store.storage_io.Diagnostics :=
  match diagnosticFailure failure with
  | none => .fail .panic
  | some failureCode =>
    match output.replies with
    | [] => .div
    | reply :: rest =>
      match reply with
      | .panicked => .fail .panic
      | .nonReturning => .div
      | .written | .writeFailed | .suppressed =>
        .ok { output with
          replies := rest
          calls := output.calls ++ [⟨diagnosticPhase phase, failureCode, elapsed, budget⟩] }

end SandproofStorageRust
