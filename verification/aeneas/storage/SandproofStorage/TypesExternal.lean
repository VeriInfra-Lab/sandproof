import SandproofFoundations.Types

/- Draft external value representations, not a model of the Redis engine.
The IO schedule ranges over normal replies, definite pre-call refusal,
uncertain delivery, panic and non-return. Physical database effects are not
inferred from these values; A11/B4 must provide their separate relation. -/
open Aeneas Aeneas.Std
namespace SandproofStorageRust

abbrev core.fmt.Formatter := SandproofFoundationsRust.core.fmt.Formatter
abbrev core.fmt.Error := SandproofFoundationsRust.core.fmt.Error
abbrev core.fmt.Display := SandproofFoundationsRust.core.fmt.Display
abbrev core.fmt.rt.Argument := SandproofFoundationsRust.core.fmt.rt.Argument
abbrev core.fmt.Arguments := SandproofFoundationsRust.core.fmt.Arguments

structure core.net.socket_addr.SocketAddrV4 where
  identity : Nat

structure core.time.Duration where
  seconds : Nat
  nanoseconds : Nat

structure std.fs.File where
  identity : Nat

structure std.time.Instant where
  tick : Nat

structure core.str.iter.Lines where
  remaining : Str

structure alloc.collections.btree.map.BTreeMap (K V A : Type) where
  entries : List (K × V)

inductive LibraryReply where
  | number (value : I64)
  | text (value : String)
  | settings (value : List (String × String))
  | refusedBeforeCall
  | deliveryUnknown
  | panicked
  | nonReturning

structure LibraryRequest where
  command : String
  arguments : List (List U8)
  deadline : Nat

structure redis.connection.Connection where
  identity : Nat
  replies : List LibraryReply
  requests : List LibraryRequest

end SandproofStorageRust
