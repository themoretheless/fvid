//! Translate legacy frame identifiers into owned PCM metadata without FFI calls.
use super::*;
use crate::owned_pcm_format::Encoding;
fn describe(format: i32) -> Option<(Encoding, bool, i32)> {
    Some(match format {
        AVSampleFormat_AV_SAMPLE_FMT_U8 => (Encoding::U8, false, AVSampleFormat_AV_SAMPLE_FMT_U8),
        AVSampleFormat_AV_SAMPLE_FMT_U8P => (Encoding::U8, true, AVSampleFormat_AV_SAMPLE_FMT_U8),
        AVSampleFormat_AV_SAMPLE_FMT_S16 => {
            (Encoding::I16, false, AVSampleFormat_AV_SAMPLE_FMT_S16)
        }
        AVSampleFormat_AV_SAMPLE_FMT_S16P => {
            (Encoding::I16, true, AVSampleFormat_AV_SAMPLE_FMT_S16)
        }
        AVSampleFormat_AV_SAMPLE_FMT_S32 => {
            (Encoding::I32, false, AVSampleFormat_AV_SAMPLE_FMT_S32)
        }
        AVSampleFormat_AV_SAMPLE_FMT_S32P => {
            (Encoding::I32, true, AVSampleFormat_AV_SAMPLE_FMT_S32)
        }
        AVSampleFormat_AV_SAMPLE_FMT_S64 => {
            (Encoding::I64, false, AVSampleFormat_AV_SAMPLE_FMT_S64)
        }
        AVSampleFormat_AV_SAMPLE_FMT_S64P => {
            (Encoding::I64, true, AVSampleFormat_AV_SAMPLE_FMT_S64)
        }
        AVSampleFormat_AV_SAMPLE_FMT_FLT => {
            (Encoding::F32, false, AVSampleFormat_AV_SAMPLE_FMT_FLT)
        }
        AVSampleFormat_AV_SAMPLE_FMT_FLTP => {
            (Encoding::F32, true, AVSampleFormat_AV_SAMPLE_FMT_FLT)
        }
        AVSampleFormat_AV_SAMPLE_FMT_DBL => {
            (Encoding::F64, false, AVSampleFormat_AV_SAMPLE_FMT_DBL)
        }
        AVSampleFormat_AV_SAMPLE_FMT_DBLP => {
            (Encoding::F64, true, AVSampleFormat_AV_SAMPLE_FMT_DBL)
        }
        _ => return None,
    })
}
pub(super) fn packed(format: i32) -> i32 {
    describe(format).map_or(-1, |v| v.2)
}
pub(super) fn planar(format: i32) -> bool {
    describe(format).is_some_and(|v| v.1)
}
pub(super) fn bytes(format: i32) -> i32 {
    describe(format).map_or(0, |v| v.0.sample_bytes() as i32)
}
pub(super) fn name(format: i32) -> String {
    describe(format).map_or_else(String::new, |v| v.0.name(v.1).into())
}
