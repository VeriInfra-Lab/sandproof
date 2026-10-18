//! Call-through roots for the original process-local drain read methods.
//! No copied business implementation and no mutation/drop extraction.
#[path = "../../../controller-service/src/application/data_drain.rs"]
pub mod data_drain;

pub fn read_drained(lease: &data_drain::DataLease) -> bool {
    lease.drained()
}

pub fn read_routes_dropped(lease: &data_drain::DataLease) -> bool {
    lease.routes_dropped()
}
