//! Canonical codec and profile metadata vocabulary owned by FVid.
//! Describing a profile does not imply that its decoding tools are implemented.
#[derive(Clone, Copy, Debug)]
pub struct CodecDescriptor {
    pub name: &'static str,
    pub profiles: &'static [(i32, &'static str)],
}
impl CodecDescriptor {
    pub fn profile_name(self, profile: i32) -> Option<&'static str> {
        self.profiles
            .iter()
            .find(|(id, _)| *id == profile)
            .map(|(_, name)| *name)
    }
}
/// Profile identifiers use each codec's metadata numbering convention.
/// Unknown codecs and profiles remain unknown, without inference from names.
pub fn descriptor(name: &str) -> Option<CodecDescriptor> {
    let (name, profiles): (&'static str, &'static [(i32, &'static str)]) = match name {
        "h264" => (
            "h264",
            &[
                (66, "Baseline"),
                (578, "Constrained Baseline"),
                (77, "Main"),
                (88, "Extended"),
                (100, "High"),
                (110, "High 10"),
                (2158, "High 10 Intra"),
                (122, "High 4:2:2"),
                (2170, "High 4:2:2 Intra"),
                (144, "High 4:4:4"),
                (244, "High 4:4:4 Predictive"),
                (2292, "High 4:4:4 Intra"),
                (44, "CAVLC 4:4:4"),
                (118, "Multiview High"),
                (128, "Stereo High"),
            ],
        ),
        "hevc" => (
            "hevc",
            &[
                (1, "Main"),
                (2, "Main 10"),
                (3, "Main Still Picture"),
                (4, "Rext"),
                (6, "Multiview Main"),
                (9, "Scc"),
            ],
        ),
        "av1" => ("av1", &[(0, "Main"), (1, "High"), (2, "Professional")]),
        "vp8" => ("vp8", &[]),
        "vp9" => (
            "vp9",
            &[
                (0, "Profile 0"),
                (1, "Profile 1"),
                (2, "Profile 2"),
                (3, "Profile 3"),
            ],
        ),
        "ffv1" => ("ffv1", &[]),
        "aac" => (
            "aac",
            &[
                (1, "LC"),
                (4, "HE-AAC"),
                (28, "HE-AACv2"),
                (22, "LD"),
                (38, "ELD"),
                (0, "Main"),
                (2, "SSR"),
                (3, "LTP"),
                (41, "xHE-AAC"),
            ],
        ),
        "aac_latm" => (
            "aac_latm",
            &[
                (1, "LC"),
                (4, "HE-AAC"),
                (28, "HE-AACv2"),
                (22, "LD"),
                (38, "ELD"),
                (0, "Main"),
                (2, "SSR"),
                (3, "LTP"),
                (41, "xHE-AAC"),
            ],
        ),
        "opus" => ("opus", &[]),
        "vorbis" => ("vorbis", &[]),
        "flac" => ("flac", &[]),
        "alac" => ("alac", &[]),
        "mp3" => ("mp3", &[]),
        "mp2" => ("mp2", &[]),
        "ac3" => ("ac3", &[]),
        "eac3" => ("eac3", &[(30, "Dolby Digital Plus + Dolby Atmos")]),
        "pcm_u8" => ("pcm_u8", &[]),
        "pcm_s8" => ("pcm_s8", &[]),
        "pcm_s16le" => ("pcm_s16le", &[]),
        "pcm_s16be" => ("pcm_s16be", &[]),
        "pcm_s24le" => ("pcm_s24le", &[]),
        "pcm_s24be" => ("pcm_s24be", &[]),
        "pcm_s32le" => ("pcm_s32le", &[]),
        "pcm_s32be" => ("pcm_s32be", &[]),
        "pcm_s64le" => ("pcm_s64le", &[]),
        "pcm_s64be" => ("pcm_s64be", &[]),
        "pcm_f32le" => ("pcm_f32le", &[]),
        "pcm_f32be" => ("pcm_f32be", &[]),
        "pcm_f64le" => ("pcm_f64le", &[]),
        "pcm_f64be" => ("pcm_f64be", &[]),
        "pcm_alaw" => ("pcm_alaw", &[]),
        "pcm_mulaw" => ("pcm_mulaw", &[]),
        "rawvideo" => ("rawvideo", &[]),
        "mjpeg" => (
            "mjpeg",
            &[
                (192, "Baseline"),
                (193, "Sequential"),
                (194, "Progressive"),
                (195, "Lossless"),
                (247, "JPEG LS"),
            ],
        ),
        "png" => ("png", &[]),
        "bmp" => ("bmp", &[]),
        "tiff" => ("tiff", &[]),
        "gif" => ("gif", &[]),
        "webp" => ("webp", &[]),
        "mov_text" => ("mov_text", &[]),
        "subrip" => ("subrip", &[]),
        "ass" => ("ass", &[]),
        "ssa" => ("ssa", &[]),
        "bin_data" => ("bin_data", &[]),
        _ => return None,
    };
    Some(CodecDescriptor { name, profiles })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_identifiers_are_codec_specific_and_flags_remain_distinct() {
        let avc = descriptor("h264").unwrap();
        assert_eq!(avc.profile_name(66), Some("Baseline"));
        assert_eq!(avc.profile_name(66 | 512), Some("Constrained Baseline"));
        assert_eq!(avc.profile_name(110 | 2048), Some("High 10 Intra"));
        assert_eq!(descriptor("aac").unwrap().profile_name(1), Some("LC"));
        assert_eq!(descriptor("hevc").unwrap().profile_name(1), Some("Main"));
        assert_eq!(descriptor("opus").unwrap().profile_name(1), None);
        assert_eq!(avc.profile_name(-99), None);
        assert_eq!(avc.profile_name(i32::MAX), None);
        assert!(descriptor("not-a-codec").is_none());
    }
}
