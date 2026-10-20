import SandproofSchemaDomains.Types
import SandproofFoundations.Funs

open Aeneas Aeneas.Std
namespace SandproofSchemaDomainsRust
/- Reuse the reviewed A6 string comparison. No new library semantics or own
project equality is hidden: the project equality bodies remain generated. -/
@[rust_fun "alloc::string::{core::cmp::PartialEq<alloc::string::String, alloc::string::String>}::eq"]
abbrev alloc.string.String.Insts.CoreCmpPartialEqString.eq :=
  SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq
end SandproofSchemaDomainsRust
