//! FVid media layer. Owned operation contracts are available without libav.
//! The temporary legacy backend retains existing operations during migration.
#[cfg(feature = "legacy-ffmpeg")]
include!("legacy.rs");
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use fvid_media_info::*;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use fvid_control::{CancelFlag, CopyOptions, ProgressEvent, ProgressHook};
