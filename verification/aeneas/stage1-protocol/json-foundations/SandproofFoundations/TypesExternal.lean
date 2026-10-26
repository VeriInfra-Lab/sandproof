import Aeneas

/-! Reviewed library observations (A6-foundations-identifiers). These are
representations of library values, not project identity constructors. -/
open Aeneas Aeneas.Std
namespace SandproofFoundationsRust

@[rust_type "core::str::iter::Bytes"]
structure core.str.iter.Bytes where
  remaining : List U8

@[rust_type "core::num::error::ParseIntError"]
inductive core.num.error.ParseIntError where
  | invalid

-- Only the default string-formatting path is observed by the frozen roots.
-- Keep the buffer and callback result; do not assume an arbitrary Display
-- callback is pure, successful, or ignored.
@[rust_type "core::fmt::Formatter"]
structure core.fmt.Formatter where
  output : String

@[rust_type "core::fmt::Error"]
inductive core.fmt.Error where
  | error

@[rust_trait "core::fmt::Display"]
structure core.fmt.Display (Self : Type) where
  fmt : Self → core.fmt.Formatter →
    Result (core.result.Result Unit core.fmt.Error × core.fmt.Formatter)

-- Finite JSON leaf observations, not a full parser or a custom Drop model.
structure core.fmt.Debug (Self : Type) where
  fmt : Self → core.fmt.Formatter →
    Result (core.result.Result Unit core.fmt.Error × core.fmt.Formatter)
structure core.error.Error (Self : Type) where
  fmtDebugInst : core.fmt.Debug Self
  fmtDisplayInst : core.fmt.Display Self

inductive JsonErrorCategory where
  | data | syntax | io | eof
structure serde_json.error.Error where
  message : String
  category : JsonErrorCategory
  line : Nat
  column : Nat


-- The only retained use of f64 is an inert Unexpected::Float payload.
-- Preserve all 64 IEEE-754 bits; no float arithmetic or formatting is modeled.
abbrev F64 := BitVec 64

structure core.fmt.rt.Argument where
  render : core.fmt.Formatter →
    Result (core.result.Result Unit core.fmt.Error × core.fmt.Formatter)

structure core.fmt.Arguments where
  bytecode : List U8
  arguments : List core.fmt.rt.Argument

end SandproofFoundationsRust
