macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name(pub u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn value(self) -> u64 {
                self.0
            }
        }
    };
}

identifier!(TenantId);
identifier!(SandboxId);
identifier!(VmId);
identifier!(FsId);
identifier!(SnapshotId);
identifier!(NetSlotId);
identifier!(OperationId);
identifier!(PolicyDigest);
identifier!(ManifestDigest);
identifier!(ResultDigest);

pub type Epoch = u64;
pub type Generation = u64;
pub type AttemptNo = u64;
