#![forbid(unsafe_code)]
//! Center business application. Redis remains the sole global authority;
//! hosts retain their original local controller and independent execution.
//! SDK parsing and pagination are reused rather than copied into this crate.

mod application;
mod contracts;
#[cfg(unix)]
mod gateway;
mod projection;
mod sdk;
mod token;
mod transport;
#[cfg(unix)]
pub use gateway::gateway_router;
#[cfg(unix)]
pub mod deployment;
#[cfg(unix)]
pub mod router;

pub use application::CenterApplication;
pub use contracts::{AuthorityPort, CenterConfig, HostPort, PublicTemplate, TemplateProvider};
pub use projection::DataRoute;
pub use sdk::CenterPort;
pub use transport::{RegisteredHost, RegisteredHosts};

#[cfg(test)]
mod tests;
