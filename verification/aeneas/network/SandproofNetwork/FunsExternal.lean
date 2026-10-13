import SandproofNetwork.Types

/-! Fixed standard-library models; correspondence to both reviewed Rust
libraries is dependency A6-network-stdlib, not an internally proved fact.
Own CIDR and policy algorithms remain real generated definitions. -/
open Aeneas Aeneas.Std
namespace SandproofNetworkRust

namespace Ipv4Library

def decimalValue (bytes : List U8) : Nat :=
  bytes.foldl (fun value byte => 10 * value + (byte.val - 48)) 0

def parseOctet (bytes : List U8) : Option Nat :=
  if bytes.length = 0 ∨ bytes.length > 3 then none
  else if bytes.length > 1 ∧ bytes.head?.map UScalar.val = some 48 then none
  else if bytes.all (fun byte => decide (48 ≤ byte.val ∧ byte.val ≤ 57)) then
    let value := decimalValue bytes
    if value < 256 then some value else none
  else none

def parseBits (bytes : List U8) : Option U32 := do
  match bytes.splitOn 46#u8 with
  | [a, b, c, d] =>
    let a ← parseOctet a
    let b ← parseOctet b
    let c ← parseOctet c
    let d ← parseOctet d
    let n := a * 2^24 + b * 2^16 + c * 2^8 + d
    if h : n < 2^32 then some (U32.ofNatCore n h) else none
  | _ => none

def stringBytes (value : String) : List U8 :=
  value.toByteArray.toList.map (fun byte =>
    U8.ofNatCore byte.toNat (by cases byte; simp; omega))

def StringRepresentable (value : String) : Prop :=
  (stringBytes value).length ≤ Usize.max

instance (value : String) : Decidable (StringRepresentable value) :=
  inferInstanceAs (Decidable (_ ≤ _))

end Ipv4Library

@[simp, rust_fun "alloc::string::{core::cmp::PartialEq<alloc::string::String, alloc::string::String>}::eq"]
def alloc.string.String.Insts.CoreCmpPartialEqString.eq
    (left right : String) : Result Bool := .ok (decide (left = right))

@[rust_fun "alloc::string::{core::ops::deref::Deref<alloc::string::String, str>}::deref"]
def alloc.string.String.Insts.CoreOpsDerefDerefStr.deref (value : String) : Result Str :=
  if h : Ipv4Library.StringRepresentable value then
    .ok (.from (Ipv4Library.stringBytes value) h)
  else .fail .maximumSizeExceeded

@[simp, rust_fun "core::str::{str}::parse"]
def core.str.Str.parse {T E : Type} (dictionary : core.str.traits.FromStr T E)
    (value : Str) : Result (core.result.Result T E) := dictionary.from_str value

@[simp, rust_fun "core::net::parser::{core::str::traits::FromStr<core::net::ip_addr::Ipv4Addr, core::net::parser::AddrParseError>}::from_str"]
def core.net.ip_addr.Ipv4Addr.Insts.CoreStrTraitsFromStrAddrParseError.from_str
    (value : Str) : Result (core.result.Result core.net.ip_addr.Ipv4Addr core.net.parser.AddrParseError) :=
  .ok (match Ipv4Library.parseBits value.val with
    | some bits => .Ok ⟨bits⟩
    | none => .Err .invalidIpv4)

@[simp, rust_fun "core::net::ip_addr::{core::convert::From<u32, core::net::ip_addr::Ipv4Addr>}::from"]
def U32.Insts.CoreConvertFromIpv4Addr.from
    (value : core.net.ip_addr.Ipv4Addr) : Result U32 := .ok value.bits

@[simp, rust_fun "alloc::string::{core::clone::Clone<alloc::string::String>}::clone"]
def alloc.string.String.Insts.CoreCloneClone.clone (value : String) : Result String := .ok value

@[rust_fun "core::option::{core::clone::Clone<core::option::Option<@T>>}::clone"]
def core.option.Option.Insts.CoreCloneClone.clone {T : Type} (dictionary : core.clone.Clone T)
    (value : Option T) : Result (Option T) :=
  match value with
  | none => .ok none
  | some item => do
      let copied ← dictionary.clone item
      .ok (some copied)

end SandproofNetworkRust
