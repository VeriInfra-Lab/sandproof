#![forbid(unsafe_code)]
//! Center business application. Redis remains the sole global authority;
//! hosts retain their original local controller and independent execution.
//! SDK parsing and pagination are reused rather than copied into this crate.

mod application;
pub mod business;
mod contracts;
mod create_binding;
#[cfg(unix)]
mod gateway;
mod lifecycle;
mod lifecycle_dispatch;
mod private_gate;
mod projection;
mod public_snapshot;
mod publication_driver;
mod sdk;
mod token;
mod transport;
#[cfg(unix)]
pub use gateway::{
    DataRouteAuthorizer, gateway_router, gateway_router_with_authorizer, gateway_router_with_builds,
};
#[cfg(unix)]
pub mod deployment;
#[cfg(unix)]
pub mod router;

pub use application::CenterApplication;
pub use contracts::{
    AuthorityPort, CenterConfig, CommittedDriver, HostPort, NodeObservationPolicy, PublicTemplate,
    TemplateProvider,
};
pub use create_binding::LifecyclePolicy;
pub use private_gate::CenterBuildPort;
pub use projection::DataRoute;
pub use public_snapshot::PublicSnapshot;
pub use sdk::CenterPort;
pub use transport::{RegisteredHost, RegisteredHosts};

#[cfg(all(test, unix))]
#[path = "../../sandbox-internal/tests/support/tls.rs"]
mod test_tls;
#[cfg(test)]
mod tests;
