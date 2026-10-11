//! Temporary entrypoint import during the single center/host cutover.
//! The native gateway, accounting and drain have only ONE implementation.
pub(crate) use sandproof_host_service::data_gateway::{
    Gateway, GatewayListener, TransferLimits, handle, relay,
};
