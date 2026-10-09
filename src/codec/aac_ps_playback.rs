//! Playback bridge for delayed native PS AAC frames and original source timing.
//! The caller must use source timing for packet-tail trim and container edits,
//! and drain EOF before announcing completion or resetting a scheduled range.
use super::aac_ps_native::{Checkpoint as NativeCheckpoint, Frame, NativePsAacDecoder};
use crate::audio::{AudioPacket, AudioSpec, SampleFormat};
#[derive(Clone, Debug, PartialEq, Eq)]
struct Source {
    index: u64,
    pts: i64,
    duration: u64,
}
pub struct PsAacDecoder {
    decoder: NativePsAacDecoder,
    pending: Option<Source>,
    failed: bool,
}
#[derive(Clone)]
pub struct Checkpoint {
    native: NativeCheckpoint,
    pending: Option<Source>,
}
/// Full decoded PCM and timing of the packet it actually belongs to.
/// Duration is a container presentation window, not the decoded sample count.
pub struct DecodedFrame {
    pub packet: AudioPacket,
    pub frame_index: u64,
    pub source_pts: i64,
    pub source_duration: u64,
}
impl PsAacDecoder {
    pub fn new(configuration: &[u8], sample_rate: u32, channels: u16) -> crate::Result<Self> {
        let asc = crate::codec::config::aac_specific_config(configuration)?;
        let decoder = NativePsAacDecoder::new(asc).map_err(|e| crate::invalid(&e.0))?;
        if decoder.sample_rate() != sample_rate || u16::from(decoder.channels()) != channels {
            return Err(crate::invalid(
                "PS AAC configuration disagrees with container sample rate or channels",
            ));
        }
        Ok(Self {
            decoder,
            pending: None,
            failed: false,
        })
    }
    /// Construct a candidate for a reader that has verified actual in-band PS.
    /// The native decoder still rejects explicit disable flags and requires PS
    /// payload presence before a successful EOF drain.
    pub fn new_with_in_band_ps(
        configuration: &[u8],
        sample_rate: u32,
        channels: u16,
    ) -> crate::Result<Self> {
        let asc = crate::codec::config::aac_specific_config(configuration)?;
        let decoder = NativePsAacDecoder::new_with_in_band_ps(asc, sample_rate)
            .map_err(|e| crate::invalid(&e.0))?;
        if channels != 2 {
            return Err(crate::invalid(
                "in-band PS requires negotiated stereo layout",
            ));
        }
        Ok(Self {
            decoder,
            pending: None,
            failed: false,
        })
    }
    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.decoder.sample_rate(),
            channels: 2,
            format: SampleFormat::F32,
        }
    }
    pub fn pending_frame_index(&self) -> Option<u64> {
        self.pending.as_ref().map(|p| p.index)
    }
    pub fn checkpoint(&self) -> Option<Checkpoint> {
        (!self.failed).then(|| Checkpoint {
            native: self.decoder.checkpoint(),
            pending: self.pending.clone(),
        })
    }
    pub fn restore(&mut self, state: &Checkpoint) -> crate::Result<()> {
        self.decoder
            .restore(&state.native)
            .map_err(|e| crate::invalid(&e.0))?;
        self.pending = state.pending.clone();
        self.failed = false;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.decoder.reset();
        self.pending = None;
        self.failed = false;
    }
    fn frame(&self, output: Option<Frame>) -> crate::Result<Option<DecodedFrame>> {
        let Some(output) = output else {
            return Ok(None);
        };
        let source = self
            .pending
            .as_ref()
            .ok_or_else(|| crate::invalid("PS AAC output has no source timestamp"))?;
        if source.index != output.frame_index {
            return Err(crate::invalid(
                "PS AAC output/source frame identity mismatch",
            ));
        }
        Ok(Some(DecodedFrame {
            packet: AudioPacket {
                data: output.pcm.iter().flat_map(|v| v.to_le_bytes()).collect(),
                pts: source.pts.max(0) as u64,
                timebase_num: 1,
                timebase_den: output.sample_rate,
            },
            frame_index: output.frame_index,
            source_pts: source.pts,
            source_duration: source.duration,
        }))
    }
    /// Decode a full access unit. Output belongs to the preceding packet, whose
    /// signed source timestamp and own duration are returned unchanged.
    pub fn decode(
        &mut self,
        data: &[u8],
        pts: i64,
        duration: u64,
    ) -> crate::Result<Option<DecodedFrame>> {
        if self.failed {
            return Err(crate::invalid(
                "PS AAC decoder requires reset after an error",
            ));
        }
        let native = self.decoder.checkpoint();
        let output = match self.decoder.decode(data) {
            Ok(output) => output,
            Err(e) => {
                self.failed = true;
                return Err(crate::invalid(&format!("PS AAC decode: {e}")));
            }
        };
        let output = match self.frame(output) {
            Ok(output) => output,
            Err(e) => {
                self.decoder
                    .restore(&native)
                    .map_err(|e| crate::invalid(&e.0))?;
                self.failed = true;
                return Err(e);
            }
        };
        let index = self
            .decoder
            .pending_frame_index()
            .ok_or_else(|| crate::invalid("PS AAC accepted input without a pending frame"))?;
        self.pending = Some(Source {
            index,
            pts,
            duration,
        });
        Ok(output)
    }
    /// Drain the last original packet. Its timing survives EOF and checkpoint replay.
    pub fn finish(&mut self) -> crate::Result<Option<DecodedFrame>> {
        if self.failed {
            return Err(crate::invalid(
                "PS AAC decoder requires reset after an error",
            ));
        }
        // An EOF seek may construct a decoder without feeding any packet.
        // There is no candidate stream to validate and no delayed frame to drain.
        // Consumed candidates still have pending source metadata and must pass
        // the native in-band PS presence check below.
        if self.pending.is_none() {
            return Ok(None);
        }
        let native = self.decoder.checkpoint();
        let output = match self.decoder.finish() {
            Ok(output) => output,
            Err(e) => {
                self.failed = true;
                return Err(crate::invalid(&format!("PS AAC EOF: {e}")));
            }
        };
        let output = match self.frame(output) {
            Ok(output) => output,
            Err(e) => {
                self.decoder
                    .restore(&native)
                    .map_err(|e| crate::invalid(&e.0))?;
                self.failed = true;
                return Err(e);
            }
        };
        self.pending = None;
        Ok(output)
    }
}

impl crate::audio::AudioDecode for PsAacDecoder {
    fn decode_packet(
        &mut self,
        data: &[u8],
        pts: i64,
        duration: u64,
    ) -> crate::Result<Option<crate::audio::DecodedAudio>> {
        self.decode(data, pts, duration).map(|out| {
            out.map(|f| crate::audio::DecodedAudio {
                packet: f.packet,
                source_pts: f.source_pts,
                source_duration: f.source_duration,
            })
        })
    }
    fn finish_packet(&mut self) -> crate::Result<Option<crate::audio::DecodedAudio>> {
        self.finish().map(|out| {
            out.map(|f| crate::audio::DecodedAudio {
                packet: f.packet,
                source_pts: f.source_pts,
                source_duration: f.source_duration,
            })
        })
    }
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let pts = i64::try_from(pts).map_err(|_| crate::invalid("PS AAC timestamp overflow"))?;
        self.decode(data, pts, duration)
            .map(|out| out.map(|f| f.packet))
    }
    fn reset(&mut self) {
        PsAacDecoder::reset(self);
    }
    fn checkpoint(&self) -> Option<crate::audio::AudioCheckpoint> {
        PsAacDecoder::checkpoint(self).map(crate::audio::AudioCheckpoint::PsAac)
    }
    fn restore(&mut self, state: &crate::audio::AudioCheckpoint) -> crate::Result<()> {
        let crate::audio::AudioCheckpoint::PsAac(state) = state else {
            return Err(crate::invalid("PS AAC checkpoint codec mismatch"));
        };
        PsAacDecoder::restore(self, state)
    }
}
