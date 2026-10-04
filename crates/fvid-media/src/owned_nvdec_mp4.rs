//! Owned MP4 packet input for direct AVC NVDEC. Media and movie clocks stay distinct.
use crate::owned_mp4::{Limits, Mp4Reader, Sample, Track};
use crate::owned_nvdec_avc_decoder::{AvcNvdecDecoder, DecodedAvc};
use fvid_codecs::codec::{
    avc::{Pps, Sps},
    config::AvcConfig,
};
use std::io::{Read, Seek};

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
    use super::*;
    use std::io::Cursor;
    const IPB: &[u8] =
        include_bytes!("../../../tests/fixtures/playback-errors/avc-multislice-ipb.mp4");
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
