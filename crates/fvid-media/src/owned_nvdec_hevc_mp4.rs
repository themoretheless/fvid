//! Own MP4 sample input for direct HEVC NVDEC, preserving both media/movie clocks.
use crate::owned_mp4::{Limits, Mp4Reader, Sample, Track};
use crate::owned_nvdec_hevc_decoder::{DecodedHevc, HevcNvdecDecoder};
use crate::owned_nvdec_mp4::{self, Presentation};
use fvid_codecs::codec::{config::HevcConfig, hevc_pps::Pps, hevc_sps::Sps};
use std::io::{Read, Seek};

pub struct DecodedHevcPacket {
    /// None for a suppressed RASL picture, distinct from end-of-input.
    pub frame: Option<DecodedHevc>,
    pub index: usize,
    pub sample: Sample,
    pub media_timescale: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use fvid_codecs::codec::hevc_decoder::HevcDecoder;
    use std::io::Cursor;
    const MAIN: &[u8] = include_bytes!("../../../tests/fixtures/hevc/main-ipb.mp4");
    const MAIN10: &[u8] = include_bytes!("../../../tests/fixtures/hevc/main10-ipb.mp4");
    #[test]
    fn synthetic_hevc_edits_keep_blanks_and_repeated_b_picture_occurrences() {
        for (bytes, depth) in [
            (
                include_bytes!("../../../tests/fixtures/playback-errors/hevc-cuda-edit-repeat.mp4")
                    .as_slice(),
                8,
            ),
            (
                include_bytes!(
                    "../../../tests/fixtures/playback-errors/hevc-main10-cuda-edit-repeat.mp4"
                )
                .as_slice(),
                10,
            ),
        ] {
            let mut input = HevcMp4Input::open(Cursor::new(bytes), Limits::default()).unwrap();
            assert_eq!(input.bit_depth(), depth);
            let mut decoder =
                HevcDecoder::from_configuration(&input.track().configuration, 64 << 20).unwrap();
            let mut packet = Vec::new();
            while input.read_next(&mut packet).unwrap().is_some() {
                assert!(decoder.decode_packet(&packet).unwrap().is_some());
            }
            input.qualify_packets().unwrap();
            assert_eq!(
                (input.movie_timescale(), input.track().timescale),
                (30, 15360)
            );
            let events = input.movie_presentations(100).unwrap();
            assert_eq!(events.len(), 14);
            let blanks: Vec<_> = events
                .iter()
                .filter(|event| event.sample.is_none())
                .collect();
            assert_eq!(blanks.len(), 2);
            assert!(blanks.iter().all(|event| event.end - event.start == 1536));
            let first: Vec<_> = events.iter().filter(|event| event.range == 1).collect();
            let repeated: Vec<_> = events.iter().filter(|event| event.range == 3).collect();
            assert_eq!(first.len(), 6);
            assert_eq!(repeated.len(), 6);
            for (a, b) in first.iter().zip(&repeated) {
                assert_eq!(a.sample, b.sample);
                assert_eq!(b.start - a.start, 4608);
                assert_eq!(a.end - a.start, 512);
            }
            assert!(events.windows(2).all(|pair| pair[0].end <= pair[1].start));
            assert_eq!(events.last().unwrap().end, 9216);
        }
    }
    #[test]
    fn own_hevc_input_qualifies_rewinds_and_preserves_all_sample_clocks() {
        for (bytes, depth) in [(MAIN, 8), (MAIN10, 10)] {
            let mut input = HevcMp4Input::open(Cursor::new(bytes), Limits::default()).unwrap();
            assert_eq!(input.bit_depth(), depth);
            input.qualify_packets().unwrap();
            assert_eq!(input.next, 0);
            let mut software =
                HevcDecoder::from_configuration(&input.track().configuration, 64 << 20).unwrap();
            let mut packet = Vec::new();
            let mut pts = Vec::new();
            for index in 0..input.packet_count() {
                let expected = input.track().samples.get(index).unwrap();
                let sample = input.read_next(&mut packet).unwrap().unwrap();
                assert_eq!(
                    (
                        sample.dts,
                        sample.pts,
                        sample.duration,
                        sample.sync,
                        sample.size
                    ),
                    (
                        expected.dts,
                        expected.pts,
                        expected.duration,
                        expected.sync,
                        expected.size
                    )
                );
                assert_eq!(packet.len(), sample.size as usize);
                assert!(software.decode_packet(&packet).unwrap().is_some());
                pts.push(sample.pts);
            }
            assert!(pts.windows(2).any(|p| p[0] > p[1]));
            assert!(input.read_next(&mut packet).unwrap().is_none());
            let order = input.media_presentation_order().unwrap();
            assert!(order.windows(2).all(|p| pts[p[0]] <= pts[p[1]]));
            assert!(!input.movie_presentations(1000).unwrap().is_empty());
            assert!(input.movie_presentations(0).is_err());
            input.qualify_packets().unwrap();
            assert_eq!(input.read_next(&mut packet).unwrap().unwrap().dts, 0);
            let metadata = input.video_metadata();
            assert_eq!(metadata.options.video.unwrap().crop, input.sps.crop);
        }
    }
    #[test]
    fn packet_limits_and_wrong_codecs_refuse_before_opening_driver() {
        assert!(
            HevcMp4Input::open(
                Cursor::new(include_bytes!(
                    "../../../tests/fixtures/playback-errors/control.mp4"
                )),
                Limits::default()
            )
            .is_err()
        );
        let mut input = HevcMp4Input::open(
            Cursor::new(MAIN),
            Limits {
                packet_bytes: 128,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(input.qualify_packets().is_err());
        assert_eq!(input.next, 0);
        assert!(!input.failed);
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA HEVC Main/Main10 NVDEC"]
    fn own_hevc_mp4_packets_decode_on_nvidia_without_libav() {
        for bytes in [include_bytes!("../../../tests/fixtures/playback-errors/cuda-hevc.mp4").as_slice(), include_bytes!("../../../tests/fixtures/playback-errors/cuda-hevc-main10.mp4").as_slice()] {
            let mut source = HevcMp4Input::open(Cursor::new(bytes), Limits::default()).unwrap();
            source.qualify_packets().unwrap();
            let count = source.packet_count();
            let mut decoder = source.create_decoder(0, 32, 2).unwrap();
            let mut scratch = Vec::new();
            let mut decoded = 0;
            while let Some(packet) = source.decode_next(&mut decoder, &mut scratch).unwrap() {
                assert_eq!(packet.index, decoded);
                assert_eq!(
                    packet.sample.pts,
                    source.track().samples.get(decoded).unwrap().pts
                );
                let frame = packet.frame.expect("synthetic IPB has no suppressed RASL");
                let mapped = decoder.map(&frame).unwrap();
                assert_eq!((mapped.width, mapped.height), source.coded_dimensions());
                decoder.unmap(mapped.slot).unwrap();
                decoded += 1;
            }
            assert_eq!(decoded, count);
            decoder.close().unwrap();
        }
    }
}
pub struct HevcMp4Input<R> {
    reader: Mp4Reader<R>,
    video: usize,
    sps: Sps,
    pps: Pps,
    length_size: u8,
    max_packet_bytes: usize,
    next: usize,
    failed: bool,
}
impl<R: Read + Seek> HevcMp4Input<R> {
    pub fn open(input: R, limits: Limits) -> Result<Self, String> {
        let max_packet_bytes = limits.packet_bytes;
        let reader = Mp4Reader::open(input, limits).map_err(|e| e.to_string())?;
        Self::from_reader(reader, max_packet_bytes)
    }
    pub(crate) fn from_reader(
        reader: Mp4Reader<R>,
        max_packet_bytes: usize,
    ) -> Result<Self, String> {
        let videos: Vec<_> = reader
            .tracks()
            .iter()
            .enumerate()
            .filter(|(_, t)| t.handler == *b"vide")
            .map(|(i, _)| i)
            .collect();
        if videos.len() != 1 {
            return Err("native HEVC MP4 input requires one video track".into());
        }
        let video = videos[0];
        let track = &reader.tracks()[video];
        if !matches!(&track.codec, b"hvc1" | b"hev1")
            || track.timescale == 0
            || track.samples.is_empty()
        {
            return Err("native HEVC MP4 input needs HEVC samples and a valid media clock".into());
        }
        let config = HevcConfig::parse(&track.configuration).map_err(|e| e.to_string())?;
        let units = |kind| {
            config
                .arrays
                .iter()
                .filter(|a| a.nal_type == kind)
                .flat_map(|a| a.units.iter().copied())
                .collect::<Vec<_>>()
        };
        let sps_units = units(33);
        let pps_units = units(34);
        if sps_units.len() != 1 || pps_units.len() != 1 {
            return Err("native HEVC MP4 input needs one active SPS/PPS pair".into());
        }
        let sps = Sps::parse(sps_units[0], max_packet_bytes).map_err(|e| e.to_string())?;
        let pps = Pps::parse(pps_units[0], &sps, max_packet_bytes).map_err(|e| e.to_string())?;
        crate::owned_nvdec_hevc::configuration(&sps, &pps)?;
        let length_size = config.length_size;
        Ok(Self {
            reader,
            video,
            sps,
            pps,
            length_size,
            max_packet_bytes,
            next: 0,
            failed: false,
        })
    }
    pub fn track(&self) -> &Track {
        &self.reader.tracks()[self.video]
    }
    pub fn movie_timescale(&self) -> u32 {
        self.reader.movie_timescale()
    }
    pub fn track_count(&self) -> usize {
        self.reader.tracks().len()
    }
    pub fn packet_count(&self) -> usize {
        self.track().samples.len()
    }
    pub fn coded_dimensions(&self) -> (u32, u32) {
        (self.sps.dimensions[0], self.sps.dimensions[1])
    }
    pub fn bit_depth(&self) -> u8 {
        self.sps.depth[0]
    }
    pub fn media_presentation_order(&self) -> Result<Vec<usize>, String> {
        owned_nvdec_mp4::media_presentation_order(self.track())
    }
    pub fn movie_presentations(&self, max_entries: usize) -> Result<Vec<Presentation>, String> {
        owned_nvdec_mp4::movie_presentations(self.track(), self.movie_timescale(), max_entries)
    }
    pub fn video_metadata(&self) -> crate::owned_nvdec_movie::MovieVideoMetadata {
        let signal = self
            .sps
            .vui
            .as_ref()
            .and_then(|v| v.signal)
            .map(|v| (v.format, v.full_range, v.colour));
        let aspect = self
            .sps
            .vui
            .as_ref()
            .and_then(|v| v.aspect_ratio)
            .and_then(|a| a.ratio());
        owned_nvdec_mp4::video_metadata(&self.reader, self.video, self.sps.crop, signal, aspect)
    }
    /// This scans every packet using the own scheduler, then restores iteration.
    /// It proves supported syntax/reference availability, not NVIDIA output.
    pub fn qualify_packets(&mut self) -> Result<(), String> {
        self.rewind_packets();
        let result = crate::owned_nvdec_hevc_decoder::qualify_packets(
            self.sps.clone(),
            self.pps.clone(),
            self.length_size,
            self.max_packet_bytes,
            |packet| self.read_next(packet).map(|s| s.is_some()),
        );
        self.rewind_packets();
        result
    }
    pub fn visible_movie_presentations(
        &mut self,
        max_entries: usize,
    ) -> Result<Vec<Presentation>, String> {
        let mut events = self.movie_presentations(max_entries)?;
        self.rewind_packets();
        let result = crate::owned_nvdec_hevc_decoder::movie_visibility(
            self.sps.clone(),
            self.pps.clone(),
            self.length_size,
            self.max_packet_bytes,
            self.packet_count(),
            |packet| self.read_next(packet).map(|s| s.is_some()),
        );
        self.rewind_packets();
        let visible = result?;
        events.retain(|event| event.sample.is_none_or(|index| visible[index]));
        Ok(events)
    }
    pub fn create_decoder(
        &self,
        ordinal: usize,
        decode_surfaces: u32,
        output_surfaces: u32,
    ) -> Result<HevcNvdecDecoder, String> {
        HevcNvdecDecoder::new(
            self.sps.clone(),
            self.pps.clone(),
            self.length_size,
            ordinal,
            decode_surfaces,
            output_surfaces,
            self.max_packet_bytes,
        )
    }
    /// Decoder must also be reopened before decoding the rewound input.
    pub fn rewind_packets(&mut self) {
        self.next = 0;
        self.failed = false;
    }
    pub fn read_next(&mut self, bytes: &mut Vec<u8>) -> Result<Option<Sample>, String> {
        if self.failed {
            return Err("HEVC MP4 input failed; rewind/reopen source and decoder".into());
        }
        if self.next >= self.packet_count() {
            return Ok(None);
        }
        let sample = self
            .track()
            .samples
            .get(self.next)
            .ok_or("missing indexed HEVC sample")?;
        match self.reader.read_packet(self.video, self.next, bytes) {
            Ok(()) => (),
            Err(error) => {
                self.failed = true;
                return Err(error.to_string());
            }
        }
        self.next += 1;
        Ok(Some(sample))
    }
    pub fn decode_next(
        &mut self,
        decoder: &mut HevcNvdecDecoder,
        scratch: &mut Vec<u8>,
    ) -> Result<Option<DecodedHevcPacket>, String> {
        let index = self.next;
        let media_timescale = self.track().timescale;
        let Some(sample) = self.read_next(scratch)? else {
            return Ok(None);
        };
        let frame = match decoder.decode(scratch) {
            Ok(frame) => frame,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        Ok(Some(DecodedHevcPacket {
            frame,
            index,
            sample,
            media_timescale,
        }))
    }
}
