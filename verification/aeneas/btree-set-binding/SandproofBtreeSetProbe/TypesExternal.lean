import Aeneas
open Aeneas Aeneas.Std
namespace SandproofBtreeSetProbeRust

/- Normal value view only.  No tree shape, addresses, allocator effects,
   abnormal rollback or comparator call-count observations are represented.
   The registered Rust calls keep using the actual standard BTreeSet. -/
structure alloc.collections.btree.set.BTreeSet (T : Type) (_A : Type) where
  entries : List T

end SandproofBtreeSetProbeRust
