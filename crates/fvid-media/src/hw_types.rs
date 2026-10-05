//! Shared CUDA operation contracts, independent of libav.
use fvid_media_info::CropRect;
use serde::Serialize;
#[derive(Clone, Debug, Default)]
pub struct HwFilterOptions {
    /// Trusted CUDA C shader over native NV12/P010 Y/U/V component codes.
    pub shader: Option<std::sync::Arc<str>>,
    /// Supply the fifth FvidSampler argument for neighborhood reads.
    pub shader_sampling: bool,
    pub crop: Option<CropRect>,
    pub horizontal_flip: bool,
    pub vertical_flip: bool,
    pub device: usize,
    /// Force a host round-trip (hwdownload then hwupload) before the device
    /// filter — same PCIe tax as FFmpeg `hwdownload,hwupload_cuda`. Counts toward
    /// `host_frame_copies`. Includes blank movie occurrences. Default path
    /// stays device-resident (`0` copies).
    pub host_bounce: bool,
    /// Half-open presentation interval in microseconds from container start.
    /// Frames outside `[from, to)` are dropped; kept frames get CFR PTS 0..N-1.
    pub interval: Option<(i64, i64)>,
    /// Compatibility option for callers requesting a shared primary context.
    /// The owned CUDA path always shares its device's primary context.
    pub share_primary_context: bool,
}

#[derive(Serialize, Debug)]
pub struct HwFilterStats {
    /// Selected filter implementation, independent of backend/encoder identity.
    pub filter: &'static str,
    pub backend: &'static str,
    pub device: String,
    pub video_frames: u64,
    pub width: u32,
    pub height: u32,
    /// Full-frame CUDA↔host transfers (two per explicit host-bounced event).
    pub host_frame_copies: u64,
    pub device_filter_passes: u64,
    pub encoder: &'static str,
    pub host_bounce: bool,
}
