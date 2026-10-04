//! Stateful native AV1 decoder. Pictures are published only after complete validation.
use super::{
    av1::Obus,
    av1_cdfs::Cdfs,
    av1_frame::Header,
    av1_metadata,
    av1_picture::{self, Picture},
    av1_sequence::{Color, Sequence},
    bits::BitReader,
};
use crate::color::hdr::HdrMetadata;
use crate::{Result, invalid};
use std::sync::Arc;
#[derive(Clone)]
pub struct Decoded {
    pub picture: Arc<Picture>,
    pub color: Color,
    pub show: bool,
}
pub struct Decoder {
    sequence: Option<Sequence>,
    initial_sequence: Option<Sequence>,
    initial_hdr: HdrMetadata,
    references: [Option<Decoded>; 8],
    showable: [bool; 8],
    reference_types: [u8; 8],
    headers: [Option<Arc<Header>>; 8],
    cdfs: [Option<Arc<Cdfs>>; 8],
    hdr: HdrMetadata,
    budget: usize,
    failed: bool,
}
impl Decoder {
    pub fn new(budget: usize) -> Self {
        Self {
            sequence: None,
            initial_sequence: None,
            initial_hdr: HdrMetadata::default(),
            references: std::array::from_fn(|_| None),
            showable: [false; 8],
            reference_types: [0; 8],
            headers: std::array::from_fn(|_| None),
            cdfs: std::array::from_fn(|_| None),
            hdr: HdrMetadata::default(),
            budget,
            failed: false,
        }
    }
    /// Seed configuration OBUs before decoding access units. Reset retains the
    /// configuration sequence and HDR metadata, but clears all frame references.
    pub fn from_configuration(record: &[u8], budget: usize) -> Result<Self> {
        let mut decoder = Self::new(budget);
        if !record.is_empty() {
            if record.len() < 4 {
                return Err(invalid("truncated AV1 codec configuration record"));
            }
            if record[0] != 0x81 {
                return Err(crate::unsupported(
                    "unsupported AV1 codec configuration version",
                ));
            }
            if !decoder.decode_packet(&record[4..])?.is_empty() {
                return Err(invalid("AV1 codec configuration contains coded frames"));
            }
            decoder.initial_sequence = decoder.sequence.clone();
            decoder.initial_hdr = decoder.hdr;
        }
        Ok(decoder)
    }
    pub fn reset(&mut self) {
        let sequence = self.initial_sequence.clone();
        let hdr = self.initial_hdr;
        *self = Self::new(self.budget);
        self.sequence = sequence.clone();
        self.initial_sequence = sequence;
        self.hdr = hdr;
        self.initial_hdr = hdr;
    }
    /// The colour the stream's own sequence header states, once one has been
    /// read. An AV1 picture says what it is coded in where the container says
    /// nothing, so a reader that wants the signal a picture is stated in asks
    /// the decoder as well as the file.
    pub fn color(&self) -> Option<&Color> {
        self.sequence.as_ref().map(|s| &s.color)
    }
    /// The static HDR light the stream's own metadata OBUs stated, since this
    /// decoder was last flushed. A volume written in-band by an encoder is
    /// often written nowhere else, so a reader that grades a picture for a
    /// panel asks the coding as well as the file.
    pub fn hdr(&self) -> HdrMetadata {
        self.hdr
    }
    pub fn decode_packet(&mut self, data: &[u8]) -> Result<Vec<Decoded>> {
        if self.failed {
            return Err(invalid("AV1 decoder requires reset after error"));
        }
        let result = self.decode_inner(data);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn decode_inner(&mut self, data: &[u8]) -> Result<Vec<Decoded>> {
        let mut output = Vec::new();
        for obu in Obus::new(data) {
            let obu = obu?;
            match obu.kind {
                1 => {
                    let sequence = Sequence::parse(obu.payload)?;
                    if sequence.operating_points[0].idc != 0 {
                        return Err(crate::unsupported(
                            "AV1 layered operating points not implemented",
                        ));
                    }
                    if self.sequence.as_ref().is_some_and(|s| s != &sequence) {
                        self.references.fill(None);
                        self.showable.fill(false);
                        self.headers.fill(None);
                        self.cdfs.fill(None);
                        self.hdr = HdrMetadata::default();
                    }
                    self.sequence = Some(sequence);
                }
                2 | 15 | 0 | 9..=14 => {}
                5 => {
                    // A metadata OBU this module cannot read costs the
                    // guidance, never the picture it travels with.
                    if let Some(hdr) = av1_metadata::hdr_from_obu(&obu) {
                        self.hdr.merge(hdr);
                    }
                }
                3 | 6 => {
                    if output.len() >= 8 {
                        return Err(invalid("too many AV1 frames per packet"));
                    }
                    let s = self
                        .sequence
                        .as_ref()
                        .ok_or_else(|| invalid("AV1 frame precedes sequence header"))?;
                    if obu.spatial_id != 0 {
                        return Err(crate::unsupported("AV1 spatial layering not implemented"));
                    }
                    if !s.reduced_header && obu.payload.first().is_some_and(|v| v & 128 != 0) {
                        let b = &mut BitReader::new(obu.payload);
                        b.bit()?;
                        let index = b.read(3)? as usize;
                        if s.decoder_model.is_some() || s.frame_id_bits.is_some() {
                            return Err(crate::unsupported(
                                "AV1 show-existing timing/frame IDs not implemented",
                            ));
                        }
                        if !self.showable[index] {
                            return Err(invalid("AV1 reference is not showable"));
                        }
                        if obu.kind != 3 {
                            return Err(invalid("AV1 show-existing requires frame header OBU"));
                        }
                        if !b.bit()? {
                            return Err(invalid("missing AV1 frame header trailing bit"));
                        }
                        while b.remaining() > 0 {
                            if b.bit()? {
                                return Err(invalid("nonzero AV1 frame header trailing bits"));
                            }
                        }
                        let mut decoded = self.references[index]
                            .clone()
                            .ok_or_else(|| invalid("missing AV1 reference"))?;
                        decoded.show = true;
                        if self.reference_types[index] == 0 {
                            self.references.fill(Some(decoded.clone()));
                            let header = self.headers[index].clone();
                            self.headers.fill(header);
                            let cdf = self.cdfs[index].clone();
                            self.cdfs.fill(cdf);
                            self.reference_types.fill(0);
                            self.showable.fill(false);
                        }
                        output.push(decoded);
                    } else {
                        if obu.kind != 6 {
                            return Err(crate::unsupported(
                                "AV1 separate frame header/tile groups not implemented",
                            ));
                        }
                        let headers = std::array::from_fn(|i| self.headers[i].as_deref());
                        let h = Header::parse(
                            s,
                            obu.payload,
                            obu.temporal_id,
                            obu.spatial_id,
                            &headers,
                        )?;
                        let initial = if h.primary_reference == 7 {
                            None
                        } else {
                            self.cdfs[h.references[h.primary_reference]].as_deref()
                        };
                        let distances = std::array::from_fn(|i| {
                            if i == 0 || s.order_hint_bits == 0 {
                                0
                            } else {
                                let hint = headers[h.references[i - 1]].map_or(0, |r| r.order_hint);
                                let diff = hint.wrapping_sub(h.order_hint) as i32;
                                let shift = 32 - s.order_hint_bits;
                                (diff << shift) >> shift
                            }
                        });
                        let mut seen = std::collections::HashSet::new();
                        let mut retained = 0usize;
                        for frame in self.references.iter().flatten().chain(output.iter()) {
                            if seen.insert(Arc::as_ptr(&frame.picture)) {
                                let bytes = frame
                                    .picture
                                    .planes
                                    .iter()
                                    .map(|p| p.samples.len() * 2)
                                    .sum::<usize>();
                                retained = retained
                                    .checked_add(bytes + 256_000)
                                    .ok_or_else(|| invalid("AV1 memory accounting overflow"))?;
                            }
                        }
                        let working_budget =
                            self.budget.checked_sub(retained).ok_or_else(|| {
                                invalid("AV1 reference pictures exceed memory budget")
                            })?;
                        let refs = std::array::from_fn(|i| {
                            self.references[i].as_ref().map(|r| r.picture.as_ref())
                        });
                        let (picture, cdf) = av1_picture::decode(
                            s,
                            &h,
                            &[&obu.payload[h.header_bytes..]],
                            working_budget,
                            initial,
                            refs,
                            distances,
                        )?;
                        let saved_header = Arc::new(h.clone());
                        let saved_cdf = Arc::new(cdf);
                        let decoded = Decoded {
                            picture: Arc::new(picture),
                            color: s.color.clone(),
                            show: h.show,
                        };
                        for i in 0..8 {
                            if h.refresh_flags & (1 << i) != 0 {
                                self.references[i] = Some(decoded.clone());
                                self.headers[i] = Some(saved_header.clone());
                                self.cdfs[i] = Some(saved_cdf.clone());
                                self.showable[i] = h.showable;
                                self.reference_types[i] = h.frame_type;
                            }
                        }
                        output.push(decoded);
                    }
                    if output.len() > 8 {
                        return Err(invalid("too many AV1 frames per packet"));
                    }
                }
                8 => return Err(crate::unsupported("AV1 tile lists not implemented")),
                _ => return Err(invalid("unsupported AV1 OBU ordering or tile list")),
            }
        }
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequences_tiles_inter_prediction_and_high_depth_match_oracle() {
        for (name, bytes, expected) in [
            (
                "inter-lossless",
                &include_bytes!("../../tests/fixtures/av1/inter-lossless.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/inter-lossless.yuv")[..],
            ),
            (
                "inter10",
                &include_bytes!("../../tests/fixtures/av1/inter10.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/inter10.yuv")[..],
            ),
            (
                "random-access",
                &include_bytes!("../../tests/fixtures/av1/random-access.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/random-access.yuv")[..],
            ),
            (
                "tiles",
                &include_bytes!("../../tests/fixtures/av1/tiles.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/tiles.yuv")[..],
            ),
            (
                "odd10",
                &include_bytes!("../../tests/fixtures/av1/odd10.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/odd10.yuv")[..],
            ),
            (
                "lossless12",
                &include_bytes!("../../tests/fixtures/av1/lossless12.obu")[..],
                &include_bytes!("../../tests/fixtures/av1/lossless12.yuv")[..],
            ),
        ] {
            let mut decoder = Decoder::new(64 << 20);
            let mut actual = Vec::new();
            let mut count = 0;
            let mut offset = 0;
            for obu in Obus::new(bytes) {
                let obu = obu.unwrap();
                let end =
                    obu.payload.as_ptr() as usize - bytes.as_ptr() as usize + obu.payload.len();
                let frames = decoder
                    .decode_packet(&bytes[offset..end])
                    .unwrap_or_else(|e| panic!("{name} frame {count}: {e}"));
                offset = end;
                for frame in frames {
                    if frame.show {
                        for (p, plane) in frame.picture.planes.iter().enumerate() {
                            let sub = usize::from(p > 0);
                            let w = (frame.picture.size[0] as usize).div_ceil(1 << sub);
                            let h = (frame.picture.size[1] as usize).div_ceil(1 << sub);
                            for y in 0..h {
                                for &v in &plane.samples[y * plane.width..y * plane.width + w] {
                                    if frame.picture.depth == 8 {
                                        actual.push(v as u8);
                                    } else {
                                        actual.extend_from_slice(&v.to_le_bytes());
                                    }
                                }
                            }
                        }
                        count += 1;
                    }
                }
            }
            assert!(count >= 2);
            assert_eq!(actual.len(), expected.len());
            for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(a, b, "{name} byte {i}");
            }
        }
    }
    #[test]
    fn damaged_streams_are_bounded_and_do_not_panic() {
        let data = include_bytes!("../../tests/fixtures/av1/random-access.obu");
        for i in (0..data.len()).step_by((data.len() / 96).max(1)) {
            let _ = Decoder::new(8 << 20).decode_packet(&data[..i]);
            let mut changed = data.to_vec();
            changed[i] ^= 0xff;
            let _ = Decoder::new(8 << 20).decode_packet(&changed);
        }
    }
    #[test]
    fn metadata_obus_hold_their_light_until_the_stream_is_flushed() {
        fn hex(bytes: &str) -> Vec<u8> {
            (0..bytes.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&bytes[at..at + 2], 16).unwrap())
                .collect()
        }
        // SVT-AV1 v4.2.0 writing `--content-light 1234,567` and
        // `--mastering-display "G(0.170,0.797)B(0.131,0.046)R(0.708,0.292)WP(0.3127,0.3290)L(1000.0,0.0001)"`
        // into one packet: the decoder remembers both halves as it walks the
        // OBUs, so a stream that writes them nowhere else still has a volume to
        // tone map from.
        let mut packet = hex("2a060104d2023780");
        packet.extend_from_slice(&hex(
            "2a1a02b53f4ac12b85cc0821890bc7500d54390003e8000000000280",
        ));
        let mut d = Decoder::new(8 << 20);
        assert!(d.hdr().is_empty());
        assert!(d.decode_packet(&packet).unwrap().is_empty());
        let hdr = d.hdr();
        assert!(hdr.mastering.unwrap().is_hdr10());
        assert_eq!((hdr.light.max_cll, hdr.light.max_fall), (1234.0, 567.0));
        // A later packet that states nothing leaves the volume standing, and a
        // flush forgets it along with the rest of the stream.
        assert!(d.decode_packet(&hex("1200")).unwrap().is_empty());
        assert_eq!(d.hdr(), hdr);
        d.reset();
        assert!(d.hdr().is_empty());
    }
    #[test]
    fn publishes_only_complete_frames_and_reset_recovers() {
        let data = include_bytes!("../../tests/fixtures/av1/ramp.obu");
        let mut d = Decoder::new(32 << 20);
        let frames = d.decode_packet(data).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].show);
        assert_eq!(frames[0].picture.size, [32, 32]);
        assert!(d.decode_packet(&data[..data.len() - 1]).is_err());
        assert!(d.decode_packet(data).is_err());
        d.reset();
        assert_eq!(d.decode_packet(data).unwrap().len(), 1);
        assert!(Decoder::new(1024).decode_packet(data).is_err());
    }
}
