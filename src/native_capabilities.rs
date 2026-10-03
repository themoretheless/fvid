//! Inventory of owned media components, independent of the legacy adapter.
//! Component presence does not imply every profile or workflow is implemented.
use crate::media_info::Capabilities;

pub fn inventory() -> Capabilities {
    let names = |values: &[&str]| {
        let mut result: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();
        result.sort();
        result.dedup();
        result
    };
    Capabilities {
        library_version: format!("FVid {}", env!("CARGO_PKG_VERSION")),
        demuxers: names(&["aac", "matroska", "mov", "wav", "yuv4mpegpipe"]),
        muxers: names(&["matroska", "mp4", "wav", "yuv4mpegpipe"]),
        decoders: names(&[
            "aac",
            "alac",
            "av1",
            "ffv1",
            "h264",
            "hevc",
            "opus",
            "vp9",
            "subrip",
            "pcm_u8",
            "pcm_s16le",
            "pcm_s16be",
            "pcm_s24le",
            "pcm_s24be",
            "pcm_s32le",
            "pcm_s32be",
            "pcm_f32le",
            "pcm_f32be",
            "pcm_f64le",
            "pcm_f64be",
        ]),
        encoders: names(&["ass", "ffv1", "pcm_f32le", "rawvideo"]),
        filters: names(&[
            "gblur",
            "avgblur",
            "boxblur",
            "chromashift",
            "crop",
            "dilation",
            "erosion",
            "eq",
            "hflip",
            "hue",
            "kirsch",
            "negate",
            "pad",
            "pixelize",
            "prewitt",
            "roberts",
            "scale",
            "scharr",
            "shuffleplanes",
            "sobel",
            "transpose",
            "unsharp",
            "vflip",
            "volume",
        ]),
    }
}
