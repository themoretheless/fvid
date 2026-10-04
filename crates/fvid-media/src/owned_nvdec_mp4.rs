//! Owned MP4 packet input for direct AVC NVDEC. Media and movie clocks stay distinct.
use crate::owned_mp4::{Limits, Mp4Reader, Sample, Track};
use crate::owned_nvdec_avc_decoder::{AvcNvdecDecoder, DecodedAvc};
use fvid_codecs::codec::{
    avc::{Pps, Sps},
    config::AvcConfig,
};
use std::io::{Read, Seek};

/// A clipped frame occurrence or explicit blank span, in media-timescale ticks
/// on the movie clock. Repeated edits can produce multiple occurrences.
#[derive(Clone, Copy, Debug)]
pub struct Presentation {
    /// Source edit range; decoder state is reset between ranges.
    pub range: usize,
    pub sample: Option<usize>,
    pub start: i64,
    pub end: i64,
}
pub struct DecodedPacket {
    pub frame: DecodedAvc,
    /// Original decode-order index; PTS can move backwards for B pictures.
    pub index: usize,
    pub sample: Sample,
    pub media_timescale: u32,
}
pub struct AvcMp4Input<R> {
    reader: Mp4Reader<R>,
    video: usize,
    sps: Sps,
    pps: Pps,
    length_size: u8,
    max_packet_bytes: usize,
    next: usize,
    failed: bool,
}
impl<R: Read + Seek> AvcMp4Input<R> {
    pub fn open(input: R, limits: Limits) -> Result<Self, String> {
        let max_packet_bytes = limits.packet_bytes;
        let reader = Mp4Reader::open(input, limits).map_err(|e| e.to_string())?;
        let videos: Vec<_> = reader
            .tracks()
            .iter()
            .enumerate()
            .filter(|(_, track)| track.handler == *b"vide")
            .map(|(index, _)| index)
            .collect();
        if videos.len() != 1 {
            return Err("native MP4 NVDEC input requires one video track".into());
        }
        let video = videos[0];
        let track = &reader.tracks()[video];
        if !matches!(&track.codec, b"avc1" | b"avc3") {
            return Err("native MP4 NVDEC input currently requires AVC".into());
        }
        if track.timescale == 0 || track.samples.is_empty() {
            return Err("native MP4 NVDEC input has no usable media clock/samples".into());
        }
        let config = AvcConfig::parse(&track.configuration).map_err(|e| e.to_string())?;
        if config.sps.len() != 1 || config.pps.len() != 1 {
            return Err("native MP4 NVDEC input needs one active SPS/PPS pair".into());
        }
        let sps = Sps::parse(config.sps[0]).map_err(|e| e.to_string())?;
        let pps = Pps::parse(config.pps[0], &sps).map_err(|e| e.to_string())?;
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
    /// Includes original edit lists. Raw sample timestamps must not be mistaken
    /// for the movie presentation timeline or silently normalized to CFR.
    pub fn track(&self) -> &Track {
        &self.reader.tracks()[self.video]
    }
    pub fn video_metadata(&self) -> crate::owned_nvdec_movie::MovieVideoMetadata {
        use crate::owned_matroska::{ColourDescription, TrackOptions, VideoMetadata};
        let track = self.track();
        let signal = self.sps.vui.as_ref().and_then(|vui| vui.video_signal);
        let mut colour = track.colour;
        if colour.primaries == 0 && colour.transfer == 0 && colour.matrix == 0 && !colour.full_range
        {
            if let Some((_, full_range, codes)) = signal {
                let [primaries, transfer, matrix] = codes.unwrap_or([2; 3]);
                colour = ColourDescription {
                    primaries,
                    transfer,
                    matrix,
                    full_range,
                };
            }
        }
        let specified = signal.is_some()
            || colour.primaries != 0
            || colour.transfer != 0
            || colour.matrix != 0
            || colour.full_range;
        let mut aspect = track.pixel_aspect;
        if matches!(track.rotation, 90 | 270) {
            aspect = (aspect.1, aspect.0);
        }
        if aspect == (1, 1) {
            if let Some((x, y)) = self.sps.vui.as_ref().and_then(|vui| vui.aspect_ratio) {
                aspect = (u32::from(x), u32::from(y));
            }
        }
        crate::owned_nvdec_movie::MovieVideoMetadata {
            file: crate::owned_matroska::FileMetadata::from_mp4(&self.reader),
            name: track.name.clone(),
            language: track.language.clone(),
            options: TrackOptions {
                rotation: track.rotation,
                video: Some(VideoMetadata {
                    crop: self.sps.crop,
                    pixel_aspect: aspect,
                    colour: specified.then_some(colour),
                    hdr: track.hdr,
                }),
                ..Default::default()
            },
        }
    }
    pub fn coded_dimensions(&self) -> (u32, u32) {
        self.sps.coded_dimensions()
    }
    pub fn movie_timescale(&self) -> u32 {
        self.reader.movie_timescale()
    }
    pub fn packet_count(&self) -> usize {
        self.track().samples.len()
    }
    /// Stable raw-media PTS order. Movie edits must be applied separately.
    pub fn media_presentation_order(&self) -> Result<Vec<usize>, String> {
        let mut order = Vec::new();
        order
            .try_reserve_exact(self.packet_count())
            .map_err(|e| e.to_string())?;
        order.extend(0..self.packet_count());
        order.sort_unstable_by_key(|index| {
            (
                self.track()
                    .samples
                    .get(*index)
                    .expect("indexed sample exists")
                    .pts,
                *index,
            )
        });
        Ok(order)
    }

    pub fn movie_presentations(&self, max_entries: usize) -> Result<Vec<Presentation>, String> {
        use crate::owned_video_timeline::{MovieEdit, map_movie_edits};
        let order = self.media_presentation_order()?;
        let track = self.track();
        let edits = map_movie_edits(
            track
                .edits
                .iter()
                .map(|edit| (edit.duration, edit.media_time)),
            track.timescale,
            self.movie_timescale(),
        )
        .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        let mut push = |entry| -> Result<(), String> {
            if result.len() >= max_entries {
                return Err("NVDEC movie presentation count exceeds limit".into());
            }
            result.try_reserve(1).map_err(|e| e.to_string())?;
            result.push(entry);
            Ok(())
        };
        if edits.is_empty() {
            for index in order {
                let sample = track.samples.get(index).unwrap();
                let end = sample
                    .pts
                    .checked_add(i64::from(sample.duration))
                    .ok_or("MP4 frame endpoint overflow")?;
                if end > sample.pts.max(0) {
                    push(Presentation {
                        sample: Some(index),
                        range: 0,
                        start: sample.pts.max(0),
                        end,
                    })?;
                }
            }
        } else {
            for (range, edit) in edits.into_iter().enumerate() {
                match edit {
                    MovieEdit::Blank {
                        movie_start,
                        movie_end,
                    } => {
                        push(Presentation {
                            sample: None,
                            range,
                            start: movie_start,
                            end: movie_end,
                        })?;
                    }
                    MovieEdit::Picture(edit) => {
                        for &index in &order {
                            let sample = track.samples.get(index).unwrap();
                            let end = sample
                                .pts
                                .checked_add(i64::from(sample.duration))
                                .ok_or("MP4 frame endpoint overflow")?;
                            let start = sample.pts.max(edit.media_start);
                            let end = end.min(edit.media_end);
                            if start < end {
                                let movie_start = edit
                                    .movie_start
                                    .checked_add(start - edit.media_start)
                                    .ok_or("MP4 movie timestamp overflow")?;
                                let movie_end = edit
                                    .movie_start
                                    .checked_add(end - edit.media_start)
                                    .ok_or("MP4 movie endpoint overflow")?;
                                push(Presentation {
                                    sample: Some(index),
                                    range,
                                    start: movie_start,
                                    end: movie_end,
                                })?;
                            }
                        }
                    }
                }
            }
        }
        Ok(result)
    }
    pub fn create_decoder(
        &self,
        ordinal: usize,
        decode_surfaces: u32,
        output_surfaces: u32,
    ) -> Result<AvcNvdecDecoder, String> {
        AvcNvdecDecoder::new(
            self.sps.clone(),
            self.pps.clone(),
            self.length_size,
            ordinal,
            decode_surfaces,
            output_surfaces,
            self.max_packet_bytes,
        )
    }
    /// Restart packet iteration. Any decoder must also be reopened before
    /// decoding after rewind; resetting the container cursor alone is insufficient.
    pub fn rewind_packets(&mut self) {
        self.next = 0;
        self.failed = false;
    }
    /// Reuses caller storage and advances only after a successful bounded read.
    pub fn read_next(&mut self, bytes: &mut Vec<u8>) -> Result<Option<Sample>, String> {
        if self.failed {
            return Err("native MP4 NVDEC input failed; reopen source and decoder".into());
        }
        let Some(sample) = self.track().samples.get(self.next) else {
            return Ok(None);
        };
        self.reader
            .read_packet(self.video, self.next, bytes)
            .map_err(|e| e.to_string())?;
        self.next += 1;
        Ok(Some(sample))
    }
    /// Read/decode in sample-table order, preserving original PTS, DTS, duration
    /// and sync metadata alongside the retained GPU decode ticket.
    pub fn decode_next(
        &mut self,
        decoder: &mut AvcNvdecDecoder,
        scratch: &mut Vec<u8>,
    ) -> Result<Option<DecodedPacket>, String> {
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
        Ok(Some(DecodedPacket {
            frame,
            index,
            sample,
            media_timescale,
        }))
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn synthetic_source_metadata_survives_owned_container_export() {
        use crate::owned_matroska::{Encoding, PacketWriter, TrackSpec};
        let mut source = AvcMp4Input::open(
            std::io::Cursor::new(
                include_bytes!(
                    "../../../tests/fixtures/playback-errors/avc-cuda-video-metadata.mp4"
                )
                .as_slice(),
            ),
            Limits::default(),
        )
        .unwrap();
        let metadata = source.video_metadata();
        assert_eq!(metadata.file.tags.title, "FVid synthetic CUDA metadata");
        assert_eq!(metadata.file.chapters[0].title, "Start");
        let video = metadata.options.video.unwrap();
        assert_eq!(metadata.options.rotation, 90);
        assert_eq!(video.pixel_aspect, (3, 2));
        let colour = video.colour.unwrap();
        assert_eq!(
            (
                colour.primaries,
                colour.transfer,
                colour.matrix,
                colour.full_range
            ),
            (1, 1, 1, false)
        );
        let (width, height) = source.coded_dimensions();
        let mut packet = Vec::new();
        let sample = source.read_next(&mut packet).unwrap().unwrap();
        let duration =
            u64::from(sample.duration) * 1_000_000_000 / u64::from(source.track().timescale);
        let tracks = [TrackSpec {
            encoding: Encoding::Avc {
                configuration: &source.track().configuration,
                width,
                height,
            },
            name: &metadata.name,
            language: "und",
        }];
        let mut output = std::io::Cursor::new(Vec::new());
        let mut writer = PacketWriter::new_with_metadata(
            &mut output,
            &tracks,
            &[metadata.options],
            &metadata.file,
        )
        .unwrap();
        writer
            .write_packet(0, 0, duration, sample.sync, &packet)
            .unwrap();
        writer.finish().unwrap();
        let read = crate::owned_webm::WebmReader::open(
            std::io::Cursor::new(output.into_inner()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(read.tags, metadata.file.tags);
        assert_eq!(read.chapters[0].title, "Start");
        assert_eq!(read.tracks[0].rotation, 90);
        assert_eq!(read.tracks[0].colour, colour);
    }

    use super::*;
    use std::io::Cursor;
    const IPB: &[u8] =
        include_bytes!("../../../tests/fixtures/playback-errors/avc-multislice-ipb.mp4");

    #[test]
    fn synthetic_movie_timeline_preserves_blanks_repeats_and_b_order() {
        const EMPTY: &[u8] =
            include_bytes!("../../../tests/fixtures/playback-errors/edit-empty-spans.mov");
        let source = AvcMp4Input::open(Cursor::new(EMPTY), Limits::default()).unwrap();
        let events = source.movie_presentations(1000).unwrap();
        let blanks: Vec<_> = events
            .iter()
            .filter(|event| event.sample.is_none())
            .collect();
        assert_eq!(blanks.len(), 2);
        assert_eq!(blanks[0].start, 0);
        assert!(blanks[0].end <= blanks[1].start);
        let first: Vec<_> = events
            .iter()
            .filter(|event| event.sample == Some(0))
            .collect();
        assert_eq!(
            first.len(),
            2,
            "same source frame must occur in both ranges"
        );
        assert!(first[0].end <= first[1].start);
        assert!(events.windows(2).all(|pair| pair[0].start <= pair[1].start));
        let expected_end = ((6000_u128 * u128::from(source.track().timescale))
            .div_ceil(u128::from(source.movie_timescale()))) as i64;
        assert_eq!(events.last().unwrap().end, expected_end);
        assert!(source.movie_presentations(1).is_err());
        let b = AvcMp4Input::open(Cursor::new(IPB), Limits::default()).unwrap();
        let events = b.movie_presentations(1000).unwrap();
        assert!(events.windows(2).all(|pair| pair[0].start <= pair[1].start));
    }
    #[test]
    fn native_packet_source_preserves_b_picture_clocks_and_payloads() {
        let mut original = Mp4Reader::open(Cursor::new(IPB), Limits::default()).unwrap();
        let mut source = AvcMp4Input::open(Cursor::new(IPB), Limits::default()).unwrap();
        assert_eq!(source.movie_timescale(), original.movie_timescale());
        let order = source.media_presentation_order().unwrap();
        assert_eq!(order.len(), source.packet_count());
        assert!(order.windows(2).all(|indices| {
            source.track().samples.get(indices[0]).unwrap().pts
                <= source.track().samples.get(indices[1]).unwrap().pts
        }));
        assert_ne!(order, (0..source.packet_count()).collect::<Vec<_>>());
        let mut software = fvid_codecs::codec::avc_decoder::AvcDecoder::new(
            &source.track().configuration,
            16 << 20,
        )
        .unwrap();
        let mut packet = Vec::new();
        let mut expected = Vec::new();
        let mut previous_pts = i64::MIN;
        let mut backwards = false;
        for index in 0..source.packet_count() {
            let sample = source.read_next(&mut packet).unwrap().unwrap();
            let reference = original.tracks()[0].samples.get(index).unwrap();
            assert_eq!(
                (sample.pts, sample.dts, sample.duration, sample.sync),
                (
                    reference.pts,
                    reference.dts,
                    reference.duration,
                    reference.sync
                )
            );
            original.read_packet(0, index, &mut expected).unwrap();
            assert_eq!(packet, expected);
            assert!(software.decode_order(&packet).unwrap().is_some());
            backwards |= sample.pts < previous_pts;
            previous_pts = sample.pts;
        }
        assert!(backwards, "fixture must contain reordered B pictures");
        assert!(source.read_next(&mut packet).unwrap().is_none());
    }
    #[test]
    fn source_retains_edit_lists_for_explicit_movie_timeline_mapping() {
        const EDIT: &[u8] =
            include_bytes!("../../../tests/fixtures/playback-errors/edit-repeat.mov");
        let original = Mp4Reader::open(Cursor::new(EDIT), Limits::default()).unwrap();
        let source = AvcMp4Input::open(Cursor::new(EDIT), Limits::default()).unwrap();
        assert!(source.track().edits.len() > 1);
        assert_eq!(
            format!("{:?}", source.track().edits),
            format!("{:?}", original.tracks()[0].edits)
        );
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVDEC"]
    fn own_mp4_packets_decode_on_nvidia_without_libav() {
        let mut source = AvcMp4Input::open(Cursor::new(IPB), Limits::default()).unwrap();
        let mut decoder = source.create_decoder(0, 32, 2).unwrap();
        let mut scratch = Vec::new();
        let mut count = 0;
        while let Some(packet) = source.decode_next(&mut decoder, &mut scratch).unwrap() {
            assert_eq!(packet.index, count);
            assert!(packet.media_timescale > 0);
            let surface = decoder.map(&packet.frame).unwrap();
            decoder.unmap(surface.slot).unwrap();
            count += 1;
        }
        assert_eq!(count, source.packet_count());
        decoder.close().unwrap();
    }
}
