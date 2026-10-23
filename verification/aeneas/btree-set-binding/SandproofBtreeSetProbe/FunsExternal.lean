import SandproofBtreeSetProbe.Types
open Aeneas Aeneas.Std
namespace SandproofBtreeSetProbeRust

namespace SetValues
def byteCompare : List U8 → List U8 → Ordering
  | [], [] => .eq
  | [], _ :: _ => .lt
  | _ :: _, [] => .gt
  | x :: xs, y :: ys =>
    if x.val < y.val then .lt
    else if y.val < x.val then .gt
    else byteCompare xs ys

def memberBy {T : Type} (order : core.cmp.Ord T) (key : T) : List T → Result Bool
  | [] => .ok false
  | x :: xs => do
    let ordering ← order.cmp x key
    if ordering = .eq then .ok true else memberBy order key xs
end SetValues

def Shared1A.Insts.CoreCmpPartialOrdShared0B.partial_cmp
    {A B : Type} (order : core.cmp.PartialOrd A B) (left : A) (right : B) :
    Result (Option Ordering) := order.partial_cmp left right

def Shared0A.Insts.CoreCmpOrd.cmp {A : Type}
    (order : core.cmp.Ord A) (left right : A) : Result Ordering := order.cmp left right

def Str.Insts.CoreCmpPartialEqStr.eq (left right : Str) : Result Bool :=
  .ok (left.val == right.val)

def Str.Insts.CoreCmpOrd.cmp (left right : Str) : Result Ordering :=
  .ok (SetValues.byteCompare left.val right.val)

def Str.Insts.CoreCmpPartialOrdStr.partial_cmp (left right : Str) :
    Result (Option Ordering) := .ok (some (SetValues.byteCompare left.val right.val))

def Pair.Insts.CoreCmpPartialEqPair.eq {U T : Type}
    (first : core.cmp.PartialEq U U) (second : core.cmp.PartialEq T T)
    (left right : U × T) : Result Bool := do
  let same ← first.eq left.1 right.1
  if same then second.eq left.2 right.2 else .ok false

def Pair.Insts.CoreCmpPartialOrdPair.partial_cmp {U T : Type}
    (first : core.cmp.PartialOrd U U) (second : core.cmp.PartialOrd T T)
    (left right : U × T) : Result (Option Ordering) := do
  let ordering ← first.partial_cmp left.1 right.1
  match ordering with
  | some .eq => second.partial_cmp left.2 right.2
  | other => .ok other

def Pair.Insts.CoreCmpOrd.cmp {U T : Type}
    (first : core.cmp.Ord U) (second : core.cmp.Ord T)
    (left right : U × T) : Result Ordering := do
  let ordering ← first.cmp left.1 right.1
  match ordering with
  | .eq => second.cmp left.2 right.2
  | other => .ok other

def alloc.collections.btree.set.BTreeSetTGlobal.new (T : Type) :
    Result (alloc.collections.btree.set.BTreeSet T Global) := .ok ⟨[]⟩

def alloc.collections.btree.set.BTreeSet.insert {T A : Type}
    (_clone : core.clone.Clone A) (order : core.cmp.Ord T)
    (seen : alloc.collections.btree.set.BTreeSet T A) (key : T) :
    Result (Bool × alloc.collections.btree.set.BTreeSet T A) := do
  let present ← SetValues.memberBy order key seen.entries
  if present then .ok (false, seen) else .ok (true, ⟨seen.entries ++ [key]⟩)

end SandproofBtreeSetProbeRust
