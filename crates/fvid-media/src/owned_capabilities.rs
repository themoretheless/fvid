//! Inventory of owned media components, independent of the legacy adapter.
//! Reports components in this library, not codecs supplied only by the root crate.
//! Component presence does not imply every profile or workflow is implemented.
use fvid_media_info::Capabilities;

pub fn capabilities() -> Capabilities {
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
            "ffv1",
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
            "drawbox", "drawgrid", "removegrain", "yaepblur", "lenscorrection", "deband", "gradfun", "bitplanenoise", "sab", "smartblur", "vignette", "curves", "hqdn3d", "tmix", "lagfun", "fade",
            "gblur",
            "bilateral",
            "avgblur",
            "boxblur",
            "chromashift",
            "colorize",
            "colorhold", "colorcontrast", "vibrance", "colorlevels", "colorchannelmixer", "exposure", "colorbalance", "colorcorrect", "cas", "grayworld",
            "monochrome",
            "lutyuv",
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
            "rotate",
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

#[cfg(test)]
mod tests {
    #[test]
    fn public_inventory_is_available_without_legacy_and_does_not_claim_root_decoders() {
        let inventory = crate::capabilities();
        assert_eq!(
            inventory.library_version,
            format!("FVid {}", env!("CARGO_PKG_VERSION"))
        );
        for names in [
            &inventory.demuxers,
            &inventory.muxers,
            &inventory.decoders,
            &inventory.encoders,
            &inventory.filters,
        ] {
            assert!(!names.is_empty());
            assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(!names.iter().any(|name| name.starts_with("lib")));
        }
        for decoder in ["aac", "alac", "ffv1", "pcm_s16le", "subrip"] {
            assert!(inventory.decoders.iter().any(|name| name == decoder));
        }
        for decoder in ["h264", "hevc", "av1", "vp9", "opus"] {
            assert!(!inventory.decoders.iter().any(|name| name == decoder));
        }
        for filter in ["colorize",
            "colorhold", "colorcontrast", "vibrance", "colorlevels", "colorchannelmixer", "exposure", "colorbalance", "colorcorrect", "cas", "grayworld", "monochrome",
            "lutyuv", "eq", "hue", "bilateral"] {
            assert!(inventory.filters.iter().any(|name| name == filter));
        }
    }
}
