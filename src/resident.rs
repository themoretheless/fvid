//! GPU-resident filter chains. Upload and download are explicit boundary operations.
//! This API does not yet import decoder surfaces or export to a hardware encoder.

/// Submitted boundary payload transfers, excluding small immutable plan uniforms.
/// Staging bytes include texture-row padding. These are API counters, not a driver trace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferStats {
    pub uploads: u64,
    pub downloads: u64,
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub upload_staging_bytes: u64,
    pub download_staging_bytes: u64,
    pub filter_passes: u64,
}

#[cfg(any(feature = "gpu", feature = "cuda"))]
mod pipeline;
#[cfg(any(feature = "gpu", feature = "cuda"))]
pub use pipeline::{GpuPipeline, ResidentFrame, UploadedFrame};

#[cfg(feature = "gpu")]
mod shader;
#[cfg(feature = "gpu")]
pub use shader::ByteShader;

/// A transform followed by an optional programmable planar-byte filter.
#[derive(Clone, Debug, Default)]
pub struct GpuStage {
    pub transform: crate::Transform,
    #[cfg(feature = "gpu")]
    pub shader: Option<ByteShader>,
    #[cfg(feature = "cuda")]
    pub cuda_shader: Option<fvid_cuda::ByteShader>,
}
impl From<crate::Transform> for GpuStage {
    fn from(transform: crate::Transform) -> Self {
        Self {
            transform,
            ..Default::default()
        }
    }
}
