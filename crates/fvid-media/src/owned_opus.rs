//! Native Opus PCM adapter. Presentation priming/padding belongs to the container.
type Result<T> = std::result::Result<T, String>;
enum Core {
    Single(fvid_opus::OpusDecoder),
    Multi(fvid_opus::OpusMSDecoder),
}
pub struct OpusDecoder {
    core: Core,
    channels: u16,
    order: Vec<usize>,
    pcm: Vec<f32>,
    failed: bool,
}
impl OpusDecoder {
    /// Conservative payload admission for the adapter's supported 48 kHz
    /// layouts. One MiB per elementary stream covers SILK/CELT state and
    /// synthesis scratch at the maximum 5760 samples, including Vec growth.
    /// Another 32 bytes per interleaved sample covers adapter/output and
    /// multistream scratch; 512 KiB covers construction and fixed storage.
    /// Encoded/rebuilt packets and container indexes are charged by the caller.
    /// This is an admission estimate, not an allocator or process RSS limit.
    pub(crate) fn decode_admission_bytes(
        configuration: &[u8],
        rate: u32,
        channels: u16,
    ) -> Result<usize> {
        let head = fvid_opus::OpusHead::parse(configuration).map_err(|e| e.to_string())?;
        if rate != 48000 || channels != u16::from(head.channel_count) {
            return Err("Opus requires its declared channel layout at 48000 Hz".into());
        }
        let streams = head
            .decoding_layout()
            .map_err(|e| e.to_string())?
            .nb_streams;
        Ok(streams * 1024 * 1024 + usize::from(channels) * 5760 * 32 + 512 * 1024)
    }
    pub fn new(configuration: &[u8], rate: u32, channels: u16) -> Result<Self> {
        let head = fvid_opus::OpusHead::parse(configuration).map_err(|e| e.to_string())?;
        if rate != 48000 || channels != u16::from(head.channel_count) {
            return Err("Opus requires its declared channel layout at 48000 Hz".into());
        }
        let layout = head.decoding_layout().map_err(|e| e.to_string())?;
        let core = if head.mapping_family == 0 {
            Core::Single(head.decoder(48000).map_err(|e| e.to_string())?)
        } else {
            let mut decoder =
                fvid_opus::OpusMSDecoder::with_layout(48000, layout).map_err(|e| e.to_string())?;
            for stream in decoder.streams_mut() {
                stream.gain_q8 = i32::from(head.output_gain_q8);
            }
            Core::Multi(decoder)
        };
        let order = match (head.mapping_family, channels) {
            (1, 3) => vec![0, 2, 1],
            (1, 5) => vec![0, 2, 1, 3, 4],
            (1, 6) => vec![0, 2, 1, 5, 3, 4],
            (1, 7) => vec![0, 2, 1, 6, 5, 3, 4],
            (1, 8) => vec![0, 2, 1, 7, 5, 6, 3, 4],
            _ => (0..usize::from(channels)).collect(),
        };
        let mut pcm = Vec::new();
        pcm.try_reserve_exact(5760 * usize::from(channels))
            .map_err(|e| e.to_string())?;
        pcm.resize(5760 * usize::from(channels), 0.0);
        Ok(Self {
            core,
            channels,
            order,
            pcm,
            failed: false,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        48000
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    fn decode_frame(&mut self, packet: &[u8]) -> Result<usize> {
        if self.failed {
            return Err("Opus decoder requires reset after an error".into());
        }
        let result = (|| {
            let expected = fvid_opus::packet::samples_48k(packet).map_err(|e| e.to_string())?;
            let frames = match &mut self.core {
                Core::Single(d) => d.decode(packet, 5760, &mut self.pcm),
                Core::Multi(d) => d.decode(packet, 5760, &mut self.pcm),
            }
            .map_err(|e| e.to_string())?;
            if frames != expected
                || self.pcm[..frames * usize::from(self.channels)]
                    .iter()
                    .any(|s| !s.is_finite())
            {
                return Err("invalid Opus decoded sample geometry".into());
            }
            Ok(frames)
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn samples(&self, frames: usize) -> impl Iterator<Item = f32> + '_ {
        self.pcm[..frames * usize::from(self.channels)]
            .chunks_exact(usize::from(self.channels))
            .flat_map(|row| self.order.iter().map(move |&channel| row[channel]))
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        let frames = self.decode_frame(packet)?;
        let mut out = Vec::new();
        out.try_reserve_exact(frames * usize::from(self.channels))
            .map_err(|e| e.to_string())?;
        out.extend(self.samples(frames));
        Ok(out)
    }
    pub fn decode_bytes(&mut self, packet: &[u8]) -> Result<Vec<u8>> {
        let frames = self.decode_frame(packet)?;
        let mut out = Vec::new();
        out.try_reserve_exact(frames * usize::from(self.channels) * 4)
            .map_err(|e| e.to_string())?;
        for value in self.samples(frames) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        Ok(out)
    }
    pub fn reset(&mut self) {
        self.failed = match &mut self.core {
            Core::Single(d) => d.reset_state().is_err(),
            Core::Multi(d) => d.reset_state().is_err(),
        };
    }
}
