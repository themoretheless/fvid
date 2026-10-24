//! Owned FLAC reconstruction adapter for library presentation timelines.
use crate::owned_matroska_audio::{invalid, Error};
pub(crate) struct FlacDecoder {
    config: fvid_codecs::flac::Config,
    mask: u32,
}
impl FlacDecoder {
    pub(crate) fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self, Error> {
        let extra = &track.codec_private;
        let info = if extra.len() == 34 {
            extra.as_slice()
        } else {
            let blocks = extra.strip_prefix(b"fLaC").unwrap_or(extra);
            if blocks.len() < 38 || blocks[0] & 127 != 0 || blocks[1..4] != [0, 0, 34] {
                return Err(invalid("FLAC requires STREAMINFO"));
            }
            &blocks[4..38]
        };
        let min_block = u16::from_be_bytes([info[0], info[1]]);
        let max_block = u16::from_be_bytes([info[2], info[3]]);
        if min_block < 16 || max_block < 16 || min_block > max_block {
            return Err(invalid("FLAC STREAMINFO states invalid block size bounds"));
        }
        let geometry = u64::from_be_bytes(info[10..18].try_into().unwrap());
        let config = fvid_codecs::flac::Config {
            sample_rate: (geometry >> 44) as u32,
            channels: ((geometry >> 41) & 7) as u16 + 1,
            bits: ((geometry >> 36) & 31) as u8 + 1,
        };
        if config.sample_rate == 0
            || !(4..=32).contains(&config.bits)
            || u64::from(config.sample_rate) != track.sample_rate
            || u64::from(config.channels) != track.channels
        {
            return Err(invalid("FLAC STREAMINFO disagrees with output geometry"));
        }
        let mask = crate::owned_flac_layout::channel_mask(extra)
            .map_err(|e| invalid(&e.to_string()))?
            .unwrap_or_else(|| channel_mask(config.channels));
        if mask.count_ones() > u32::from(config.channels) {
            return Err(invalid("FLAC channel mask exceeds stream channel count"));
        }
        Ok(Self { config, mask })
    }
    pub(crate) fn channel_mask(&self) -> u32 {
        self.mask
    }
    pub(crate) fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }
    pub(crate) fn channels(&self) -> u16 {
        self.config.channels
    }
    pub(crate) fn decode(&self, packet: &[u8]) -> Result<Vec<f32>, Error> {
        let frame =
            fvid_codecs::flac::decode(packet, self.config).map_err(|e| invalid(&e.to_string()))?;
        let scale = (1u64 << (self.config.bits - 1)) as f64;
        let mut pcm = Vec::new();
        pcm.try_reserve_exact(frame.samples.len())
            .map_err(|_| invalid("FLAC float allocation failed"))?;
        pcm.extend(
            frame
                .samples
                .into_iter()
                .map(|v| (f64::from(v) / scale) as f32),
        );
        Ok(pcm)
    }
    /// Conservative bound for 65535 samples, eight i64 planes, interleaved i32,
    /// float PCM, timeline carry/queue and decoder scratch. Encoded/index bytes
    /// are charged separately by the common Matroska admission path.
    pub(crate) const DECODE_ADMISSION_BYTES: usize = 32 * 1024 * 1024;
}

/// RFC 9639 section 9.1.3 channel order. Four-channel FLAC is quad.
pub(crate) fn channel_mask(channels: u16) -> u32 {
    crate::owned_flac_layout::default_channel_mask(channels)
}
