#![forbid(unsafe_code)]

//! Pure, deterministic sandbox-controller core.
//!
//! This crate deliberately performs no I/O.  A successful reduction returns
//! the complete next durable state and the protocol event which justified it.
//! Persistence, dispatch, clocks, and platform adapters live outside this
//! boundary.  The data model mirrors `SandboxModel.Types` and
//! `SandboxModel.State`; refinement proofs are maintained in Lean.

pub mod auxiliary;
pub mod capture;
pub mod checkpoint;
pub mod controller;
pub mod effect;
pub mod ids;
pub mod maintenance;
pub mod model;
pub mod network;
pub mod publication;
pub mod recovery_monitor;
pub mod request;
pub mod scheduler;
pub mod store;
pub mod terminal;
pub(crate) mod transition;
pub use transition::DEFAULT_RETRY_LIMIT;
pub mod worker;

pub use auxiliary::*;
pub use controller::*;
pub use effect::*;
pub use ids::*;
pub use model::*;
pub use network::*;
pub use publication::*;
pub use recovery_monitor::*;
pub use request::*;
pub use scheduler::*;
pub use store::Store;
pub use worker::*;
