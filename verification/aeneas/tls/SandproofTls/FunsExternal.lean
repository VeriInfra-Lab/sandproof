import Aeneas
import SandproofTls.Types
open Aeneas Aeneas.Std
namespace SandproofTlsRust
-- A6: exact read-only value and order observation of core::slice::first.
@[rust_fun "core::slice::{[@T]}::first"]
def core.slice.Slice.first {T : Type} (values : Slice T) : Result (Option T) :=
  .ok values.val.head?
end SandproofTlsRust
