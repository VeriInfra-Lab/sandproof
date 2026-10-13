import Aeneas

/-! Reviewed standard-library representations (A6-network-stdlib).
Only IPv4 parsing and conversion are observed by this extraction. Error details
are an observational quotient, not a model of Display/Debug or other IP parsers. -/
open Aeneas Aeneas.Std
namespace SandproofNetworkRust

@[rust_type "core::net::ip_addr::Ipv4Addr"]
structure core.net.ip_addr.Ipv4Addr where
  bits : U32

@[rust_type "core::net::parser::AddrParseError"]
inductive core.net.parser.AddrParseError where
  | invalidIpv4

end SandproofNetworkRust
