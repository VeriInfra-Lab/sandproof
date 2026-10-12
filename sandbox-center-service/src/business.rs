//! Durable template business adapter; NOT an enabled executable assembly.
//! Candidate hashing/sealing and authenticated evidence are required external
//! ports. Lifecycle and metadata always share the original authority/lock.
//! Unsettled validation pins its original center sandbox. Settlement transfers
//! the original create and exact Cleaned response into this bounded record in
//! the same CAS as the native certificate or explicit original-journal non-admission
//! evidence. Production candidate supply/cache and the final entry still need
//! assembly; controlled tests do not attest physical VM execution.
mod authority;
mod evidence_port;
mod metadata;
mod ports;
mod private_builds;
mod publication;
mod query;
mod templates;
mod workflow;

pub use authority::SharedBusinessAuthority;
pub use metadata::{MetadataBudget, RegistryMetadata, TemplateRegistration};
pub use ports::{CandidatePort, PreparationError, ProtectedCandidate, ValidationEvidencePort};
pub use private_builds::{BuildParameters, GlobalBuildAnchor, PrivateBuildConfig};
pub use publication::{
    GlobalPublication, LocalValidationBinding, ValidationAdmission, ValidationLink,
    ValidationSettlement,
};
pub use templates::BusinessTemplates;
