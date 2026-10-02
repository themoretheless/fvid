/// Encoded packet storage is passed through without rewriting codec payloads.
#[derive(Clone, Copy)]
pub enum Encoding<'a> {
    /// Interleaved finite IEEE-754 float32 samples in little-endian order.
    PcmFloat32 { sample_rate: u32, channels: u16 },
    Opus { configuration: &'a [u8] },
    /// ASS header in CodecPrivate; each packet is one Matroska ASS event.
    Ass { configuration: &'a [u8] },
    /// FFV1 v1 packets contain their configuration in every keyframe.
    Ffv1V1 { width: u32, height: u32 },
    Avc {
        configuration: &'a [u8],
        width: u32,
        height: u32,
    },
    Hevc {
        configuration: &'a [u8],
        width: u32,
        height: u32,
    },
    Aac {
        configuration: &'a [u8],
        sample_rate: u32,
        channels: u16,
    },
}

/// Description of an encoded track on the caller's presentation timeline.
/// Container edit lists, rotation and gapless trimming must be mapped by a
/// higher-level remuxer. VideoMetadata carries colour/HDR and display geometry.
pub struct TrackSpec<'a> {
    pub encoding: Encoding<'a>,
    pub name: &'a str,
    pub language: &'a str,
}
