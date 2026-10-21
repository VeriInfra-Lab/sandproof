//! Pure acceptance guard used by the real original-FD copy path.
//! Fields are one Metadata observation of the same already-open descriptor;
//! this guard does not acquire a descriptor or establish writer exclusion.

#[derive(Clone, Copy, Debug)]
pub(crate) struct SnapshotFileFacts {
    pub owner: u32,
    pub mode: u32,
    pub regular: bool,
    pub links: u64,
    pub length: u64,
}

pub(crate) fn accepts(facts: SnapshotFileFacts, expected_owner: u32, max_length: u64) -> bool {
    // Keep the exact Unix group/other mask visible to review and extraction.
    let group_other_permissions = facts.mode & 0o077;
    facts.regular
        && facts.owner == expected_owner
        && group_other_permissions == 0
        && facts.links == 1
        && facts.length > 0
        && facts.length <= max_length
}
