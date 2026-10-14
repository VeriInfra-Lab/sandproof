use crate::{
    file_rpc_requests::CompressionStage, upload_body::GzipError, upload_response::GzipReadError,
};

/// A recognized gzip failure with its actual Connect decoding stage. Empty
/// collected bodies skip decompression; local budgets and arbitrary codec
/// diagnostics cannot be turned into these fixed Connect messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GzipRejection {
    stage: CompressionStage,
    reason: GzipReadError,
}
impl GzipRejection {
    pub(crate) fn observed(stage: CompressionStage, error: GzipError) -> Option<Self> {
        if stage == CompressionStage::Header
            && !matches!(error, GzipError::Header | GzipError::Truncated)
        {
            return None;
        }
        Some(Self {
            stage,
            reason: error.try_into().ok()?,
        })
    }
    pub(crate) fn parts(self) -> [&'static [u8]; 2] {
        [
            match self.stage {
                CompressionStage::Header => b"get decompressor: ",
                CompressionStage::Body => b"decompress: ",
            },
            self.reason.message(),
        ]
    }
}
