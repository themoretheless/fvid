//! Sequential AAC-LC decoding to caller-owned interleaved float32 PCM.
use super::{
    Error, NativeAacDecoder as AdtsPacketDecoder, Result, adts::StreamReader as AdtsStreamReader,
    invalid,
};
use fvid_control::{CopyOptions, ProgressEvent};
use std::{
    io::{Read, Write},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioDecodeStats {
    pub sample_frames: u64,
    pub decoded_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
}
pub(crate) struct DecodeProgress<'a> {
    options: &'a CopyOptions,
    event: ProgressEvent,
}
impl DecodeProgress<'_> {
    fn check(&self) -> Result<()> {
        if self
            .options
            .cancel
            .as_ref()
            .is_some_and(|c| c.is_cancelled())
        {
            return Err(invalid("media operation cancelled"));
        }
        crate::owned_budget::check_rss_budget(self.options).map_err(Error)
    }
    fn packet_limit_reached(&self) -> bool {
        self.options
            .max_packets
            .is_some_and(|max| self.event.packets >= max)
    }
    fn packet(&mut self, bytes: usize) -> Result<()> {
        self.event.packets = self
            .event
            .packets
            .checked_add(1)
            .ok_or_else(|| invalid("audio packet count overflow"))?;
        self.event.payload_bytes = self
            .event
            .payload_bytes
            .checked_add(bytes as u64)
            .ok_or_else(|| invalid("audio byte count overflow"))?;
        if let Some(progress) = &self.options.progress {
            progress.emit(self.event);
        }
        self.check()
    }
}

/// Decode ADTS without a file index or full PCM buffer. ADTS priming/padding is
/// retained. Ranges use ceil-rounded sample boundaries and retain preroll.
/// Packet counts include preroll; reaching the count/range stops before reading
/// the next header. Full decoding validates the complete source.
///
/// The caller owns publication and flushing; failures can leave partial PCM in
/// its writer. Progress never emits completion for a caller-owned destination.
/// Metadata mutation and aggregate allocation admission are not implemented
/// by this raw-stream API and are rejected explicitly rather than ignored.
pub fn decode_adts_pcm<R: Read>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if options.max_controlled_bytes.is_some() {
        return Err(invalid(
            "ADTS stream allocation admission is not yet implemented",
        ));
    }
    if !(options.streams.is_empty() || options.streams == [0]) {
        return Err(invalid("ADTS has only stream 0"));
    }
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(invalid("raw PCM stream cannot apply metadata mutations"));
    }
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let mut control = DecodeProgress {
        options,
        event: ProgressEvent {
            packets: 0,
            payload_bytes: 0,
            done: false,
        },
    };
    control.check()?;
    if let Some(progress) = &options.progress {
        progress.emit(control.event);
    }
    control.check()?;
    if control.packet_limit_reached() {
        return Err(invalid("audio interval contains no samples"));
    }
    let reader = AdtsStreamReader::open_with_packet_limit(source, options.max_packet_bytes)?;
    decode_adts_aac_reader_controlled(reader, output, interval, &mut control)
}
include!("stream_impl.rs");

#[cfg(test)]
mod tests {
    #[test]
    fn pcm_serialization_preserves_bits_and_handles_short_writes() {
        struct ShortWriter {
            bytes: Vec<u8>,
            calls: usize,
        }
        impl std::io::Write for ShortWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                let count = bytes.len().min(777);
                self.bytes.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let samples: Vec<f32> = (0..2051).map(|n| f32::from_bits(n * 7919)).collect();
        let expected: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut output = ShortWriter {
            bytes: Vec::new(),
            calls: 0,
        };
        super::write_pcm_samples(&mut output, &samples).unwrap();
        assert_eq!(output.bytes, expected);
        assert!(output.calls < 20);
        let calls = output.calls;
        super::write_pcm_samples(&mut output, &[]).unwrap();
        assert_eq!(output.calls, calls);
    }

    #[test]
    fn standalone_stream_api_retains_exact_clock_and_packet_prefix() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let bytes = std::fs::read(root.join("playback-errors/aac-packet-prefix.aac")).unwrap();
        let mut output = Vec::new();
        let options = fvid_control::CopyOptions {
            max_packets: Some(3),
            ..Default::default()
        };
        let stats = super::decode_adts_pcm(bytes.as_slice(), &mut output, None, &options).unwrap();
        assert_eq!(
            (
                stats.sample_frames,
                stats.decoded_frames,
                stats.sample_rate,
                stats.channels
            ),
            (3072, 3, 44100, 1)
        );
        assert_eq!(output.len(), 3072 * 4);
        assert!(
            super::decode_adts_pcm(bytes.as_slice(), &mut Vec::new(), None, &Default::default())
                .unwrap_err()
                .to_string()
                .contains("fill whole buffer")
        );
    }
}
