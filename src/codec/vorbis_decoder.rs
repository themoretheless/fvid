//! Player adapter for the independent FVid Vorbis packet decoder.

use crate::audio::{AudioPacket, AudioSpec, SampleFormat};
/// Vorbis decoder state.
pub struct VorbisDecoder {
    decoder: fvid_codecs::vorbis_audio::Decoder,
    sample_rate: u32,
    channels: u16,
}

impl VorbisDecoder {
    /// Create a new Vorbis decoder from the track's configuration.
    pub fn new(configuration: &[u8], sample_rate: u32, channels: u16) -> crate::Result<Self> {
        let (ident, setup) = fvid_codecs::vorbis_audio::configuration_headers(configuration)?;
        let ident = fvid_codecs::vorbis::Identification::parse(ident)?;
        if ident.sample_rate != sample_rate || u16::from(ident.channels) != channels {
            return Err(crate::invalid(
                "Vorbis track and identification geometry disagree",
            ));
        }
        let setup = fvid_codecs::vorbis::Setup::parse(
            setup,
            ident.channels,
            fvid_codecs::vorbis_admission::inspect(setup)?.limits,
        )?;
        // Derive workspace from the actual source geometry and largest book,
        // including the worst classification padding and flattened residue.
        let n = 1usize << ident.block_exponents[1];
        let ch = usize::from(ident.channels);
        let dimension = setup
            .codebooks
            .iter()
            .map(|b| usize::from(b.dimensions))
            .max()
            .unwrap_or(0);
        let workspace = ch * n * 32
            + n * 40
            + 64 * 1024
            + 16 * (ch * (n / 2 + dimension) + dimension + ch * n / 2);
        let decoder = fvid_codecs::vorbis_audio::Decoder::new(ident, setup, workspace)?;

        Ok(Self {
            decoder,
            sample_rate,
            channels,
        })
    }

    /// Decode a Vorbis packet and return PCM samples as interleaved f32.
    pub fn decode(
        &mut self,
        packet_data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let _ = duration;
        let samples = self.decoder.decode(packet_data)?;
        if samples.is_empty() {
            return Ok(None);
        }
        let mut sample_bytes = Vec::new();
        sample_bytes
            .try_reserve_exact(samples.len() * 4)
            .map_err(|_| crate::invalid("cannot allocate Vorbis PCM packet"))?;
        for sample in samples {
            sample_bytes.extend_from_slice(&sample.to_le_bytes());
        }

        Ok(Some(AudioPacket {
            data: sample_bytes,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Current audio specification (sample rate, channels).
    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels,
            format: SampleFormat::F32,
        }
    }

    /// Reset decoder state (e.g., after a seek).
    pub fn reset(&mut self) {
        self.decoder.reset();
    }
}

impl crate::audio::AudioDecode for VorbisDecoder {
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        VorbisDecoder::decode(self, data, pts, duration)
    }

    fn reset(&mut self) {
        VorbisDecoder::reset(self);
    }
}
