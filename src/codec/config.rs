//! Decoder configuration records and length-delimited NAL units.
//! These parsers extract codec initialization data; they do not decode samples.
use super::bits::BitReader;
use crate::{Result, invalid};

struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Bytes<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .ok_or_else(|| invalid("configuration length overflow"))?;
        let part = self
            .data
            .get(self.at..end)
            .ok_or_else(|| invalid("truncated codec configuration"))?;
        self.at = end;
        Ok(part)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn short(&mut self) -> Result<usize> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize)
    }
    fn end(&self) -> Result<()> {
        if self.at != self.data.len() {
            return Err(invalid("extra codec configuration bytes"));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct AvcConfig<'a> {
    pub profile: u8,
    pub compatibility: u8,
    pub level: u8,
    pub length_size: u8,
    pub sps: Vec<&'a [u8]>,
    pub pps: Vec<&'a [u8]>,
    pub sps_extensions: Vec<&'a [u8]>,
}
fn avc_sets<'a>(b: &mut Bytes<'a>, count: usize, kind: u8) -> Result<Vec<&'a [u8]>> {
    let mut sets = Vec::new();
    for _ in 0..count {
        let size = b.short()?;
        let nal = b.take(size)?;
        if nal.first().is_none_or(|h| h & 0x80 != 0 || h & 31 != kind) {
            return Err(invalid("invalid AVC parameter-set NAL"));
        }
        sets.push(nal);
    }
    Ok(sets)
}
impl<'a> AvcConfig<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let mut b = Bytes::new(data);
        if b.byte()? != 1 {
            return Err(invalid("unsupported avcC version"));
        }
        let profile = b.byte()?;
        let compatibility = b.byte()?;
        let level = b.byte()?;
        let lengths = b.byte()?;
        if lengths & 0xfc != 0xfc || lengths & 3 == 2 {
            return Err(invalid("invalid AVC NAL length size"));
        }
        let length_size = (lengths & 3) + 1;
        let counts = b.byte()?;
        if counts & 0xe0 != 0xe0 {
            return Err(invalid("invalid avcC reserved bits"));
        }
        let sps = avc_sets(&mut b, (counts & 31) as usize, 7)?;
        let count = b.byte()? as usize;
        let pps = avc_sets(&mut b, count, 8)?;
        let mut sps_extensions = Vec::new();
        if b.at != data.len() {
            if !matches!(profile, 100 | 110 | 122 | 144) {
                return Err(invalid("unexpected AVC profile extension"));
            }
            let chroma = b.byte()?;
            let luma = b.byte()?;
            let chroma_depth = b.byte()?;
            if chroma & 0xfc != 0xfc || luma & 0xf8 != 0xf8 || chroma_depth & 0xf8 != 0xf8 {
                return Err(invalid("invalid AVC extension reserved bits"));
            }
            let count = b.byte()? as usize;
            sps_extensions = avc_sets(&mut b, count, 13)?;
        }
        b.end()?;
        Ok(Self {
            profile,
            compatibility,
            level,
            length_size,
            sps,
            pps,
            sps_extensions,
        })
    }
}

#[derive(Debug)]
pub struct HevcArray<'a> {
    pub complete: bool,
    pub nal_type: u8,
    pub units: Vec<&'a [u8]>,
}
#[derive(Debug)]
pub struct HevcConfig<'a> {
    pub profile_space: u8,
    pub tier: bool,
    pub profile: u8,
    pub compatibility: u32,
    pub constraints: [u8; 6],
    pub level: u8,
    pub chroma_format: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub length_size: u8,
    pub arrays: Vec<HevcArray<'a>>,
}
impl<'a> HevcConfig<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let mut b = Bytes::new(data);
        let h = b.take(23)?;
        if h[0] != 1 {
            return Err(invalid("unsupported hvcC version"));
        }
        if h[13] & 0xf0 != 0xf0
            || h[15] & 0xfc != 0xfc
            || h[16] & 0xfc != 0xfc
            || h[17] & 0xf8 != 0xf8
            || h[18] & 0xf8 != 0xf8
        {
            return Err(invalid("invalid hvcC reserved bits"));
        }
        let length_size = (h[21] & 3) + 1;
        if length_size == 3 {
            return Err(invalid("invalid HEVC NAL length size"));
        }
        let mut arrays = Vec::new();
        let mut total = 0;
        for _ in 0..h[22] {
            let header = b.byte()?;
            if header & 0x40 != 0 {
                return Err(invalid("invalid HEVC array reserved bit"));
            }
            let nal_type = header & 63;
            if !matches!(nal_type, 32 | 33 | 34 | 39 | 40) {
                return Err(invalid("unsupported HEVC configuration NAL type"));
            }
            if arrays.iter().any(|a: &HevcArray| a.nal_type == nal_type) {
                return Err(invalid("duplicate HEVC configuration array"));
            }
            let count = b.short()?;
            total += count;
            if total > 4096 {
                return Err(invalid("too many HEVC configuration NALs"));
            }
            let mut units = Vec::new();
            for _ in 0..count {
                let size = b.short()?;
                let nal = b.take(size)?;
                let parsed = super::hevc_nal::NalHeader::parse(nal)?;
                if parsed.unit_type != nal_type {
                    return Err(invalid("invalid HEVC configuration NAL header"));
                }
                units.push(nal);
            }
            arrays.push(HevcArray {
                complete: header & 0x80 != 0,
                nal_type,
                units,
            });
        }
        b.end()?;
        Ok(Self {
            profile_space: h[1] >> 6,
            tier: h[1] & 32 != 0,
            profile: h[1] & 31,
            compatibility: u32::from_be_bytes(h[2..6].try_into().unwrap()),
            constraints: h[6..12].try_into().unwrap(),
            level: h[12],
            chroma_format: h[16] & 3,
            bit_depth_luma: 8 + (h[17] & 7),
            bit_depth_chroma: 8 + (h[18] & 7),
            length_size,
            arrays,
        })
    }
}

/// Borrowed NAL iterator, with one terminal error on malformed framing.
pub struct NalUnits<'a> {
    data: &'a [u8],
    length_size: usize,
}
impl<'a> NalUnits<'a> {
    pub fn new(data: &'a [u8], length_size: u8) -> Result<Self> {
        if !matches!(length_size, 1 | 2 | 4) {
            return Err(invalid("unsupported NAL length size"));
        }
        Ok(Self {
            data,
            length_size: length_size as usize,
        })
    }
}
impl<'a> Iterator for NalUnits<'a> {
    type Item = Result<&'a [u8]>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.data.is_empty() {
            return None;
        }
        let Some(header) = self.data.get(..self.length_size) else {
            self.data = &[];
            return Some(Err(invalid("truncated NAL length")));
        };
        let len = header
            .iter()
            .fold(0usize, |v, &b| (v << 8) | usize::from(b));
        let payload = &self.data[self.length_size..];
        if len == 0 || len > payload.len() {
            self.data = &[];
            return Some(Err(invalid("invalid NAL payload length")));
        }
        let (nal, rest) = payload.split_at(len);
        self.data = rest;
        Some(Ok(nal))
    }
}

fn descriptor<'a>(b: &mut Bytes<'a>) -> Result<(u8, &'a [u8])> {
    let tag = b.byte()?;
    let mut length = 0usize;
    for i in 0..4 {
        let value = b.byte()?;
        length = (length << 7) | usize::from(value & 127);
        if value & 128 == 0 {
            return Ok((tag, b.take(length)?));
        }
        if i == 3 {
            return Err(invalid("descriptor length exceeds four bytes"));
        }
    }
    unreachable!()
}
/// Extract the AAC AudioSpecificConfig from an esds full-box payload.
pub fn aac_specific_config(esds: &[u8]) -> Result<&[u8]> {
    let mut b = Bytes::new(esds);
    if b.take(4)? != [0, 0, 0, 0] {
        return Err(invalid("unsupported esds version or flags"));
    }
    let (tag, es) = descriptor(&mut b)?;
    b.end()?;
    if tag != 3 {
        return Err(invalid("expected ES descriptor"));
    }
    let mut es = Bytes::new(es);
    es.take(2)?;
    let flags = es.byte()?;
    if flags & 128 != 0 {
        es.take(2)?;
    }
    if flags & 64 != 0 {
        let n = usize::from(es.byte()?);
        es.take(n)?;
    }
    if flags & 32 != 0 {
        es.take(2)?;
    }
    let mut asc = None;
    let mut decoder_seen = false;
    while es.at < es.data.len() {
        let (tag, data) = descriptor(&mut es)?;
        if tag != 4 {
            continue;
        }
        if decoder_seen {
            return Err(invalid("duplicate decoder configuration"));
        }
        decoder_seen = true;
        let mut dc = Bytes::new(data);
        let header = dc.take(13)?;
        if header[0] != 0x40 || header[1] >> 2 != 5 || header[1] & 1 != 1 {
            return Err(invalid("not MPEG-4 audio decoder configuration"));
        }
        while dc.at < dc.data.len() {
            let (tag, data) = descriptor(&mut dc)?;
            if tag == 5 {
                if asc.replace(data).is_some() {
                    return Err(invalid("duplicate AudioSpecificConfig"));
                }
            }
        }
    }
    asc.ok_or_else(|| invalid("missing AudioSpecificConfig"))
}
const AAC_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];
#[derive(Debug, PartialEq, Eq)]
pub struct AacConfig {
    pub object_type: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_samples: u16,
    pub core_coder_delay: Option<u16>,
}
fn audio_object_type(b: &mut BitReader<'_>) -> Result<u32> {
    let n = b.read(5)?;
    if n == 31 { Ok(32 + b.read(6)?) } else { Ok(n) }
}
impl AacConfig {
    /// Parse AAC-LC metadata, including the count from an explicit program.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let (config, _) = Self::parse_with_program(data)?;
        Ok(config)
    }
    /// Preserve an explicit tagged program rather than guessing a layout from its count.
    pub fn parse_with_program(data: &[u8]) -> Result<(Self, Option<super::aac_pce::ProgramConfig>)> {
        let mut b = BitReader::new(data);
        let object_type = audio_object_type(&mut b)?;
        let index = b.read(4)? as usize;
        let sample_rate = if index == 15 {
            b.read(24)?
        } else {
            *AAC_RATES
                .get(index)
                .ok_or_else(|| invalid("reserved AAC frequency index"))?
        };
        if sample_rate == 0 {
            return Err(invalid("zero AAC frequency"));
        }
        let config = b.read(4)?;
        if object_type != 2 {
            return Err(invalid("only AAC-LC configuration is implemented"));
        }
        let mut channels = match config {
            0 => 0,
            1..=6 => config as u8,
            7 => 8,
            _ => return Err(invalid("unsupported AAC channel configuration")),
        };
        let frame_samples = if b.bit()? { 960 } else { 1024 };
        let core_coder_delay = if b.bit()? {
            Some(b.read(14)? as u16)
        } else {
            None
        };
        if b.bit()? {
            return Err(invalid("AAC extension flag is not yet supported"));
        }
        let program = if config == 0 {
            let program = super::aac_pce::ProgramConfig::read(&mut b, 0)?;
            if u32::from(program.object_type) != object_type || program.sample_rate != sample_rate {
                return Err(invalid("AAC PCE disagrees with AudioSpecificConfig"));
            }
            channels = program.channels() as u8;
            Some(program)
        } else { None };
        // Explicitly consume the common backward-compatible SBR sync extension.
        if b.remaining() >= 16 {
            if b.read(11)? != 0x2b7 {
                return Err(invalid("unsupported AAC trailing extension"));
            }
            if audio_object_type(&mut b)? != 5 || b.bit()? {
                return Err(invalid("AAC SBR decoding is not yet implemented"));
            }
        }
        while b.remaining() > 0 {
            if b.bit()? {
                return Err(invalid("nonzero AAC trailing bits"));
            }
        }
        Ok((Self {
            object_type,
            sample_rate,
            channels,
            frame_samples,
            core_coder_delay,
        }, program))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AVC: &[u8] = &[1, 66, 0, 10, 255, 225, 0, 2, 103, 128, 1, 0, 2, 104, 128];
    #[test]
    fn avc_config_and_every_truncation() {
        let c = AvcConfig::parse(AVC).unwrap();
        assert_eq!(c.length_size, 4);
        assert_eq!(c.sps, vec![&[103, 128][..]]);
        assert_eq!(c.pps, vec![&[104, 128][..]]);
        for n in 0..AVC.len() {
            assert!(AvcConfig::parse(&AVC[..n]).is_err());
        }
        let mut bad = AVC.to_vec();
        bad[4] = 254;
        assert!(AvcConfig::parse(&bad).is_err());
        bad = AVC.to_vec();
        bad[8] = 104;
        assert!(AvcConfig::parse(&bad).is_err());
    }
    #[test]
    fn hevc_arrays_and_reserved_bits() {
        let mut h = vec![0; 23];
        h[0] = 1;
        h[1] = 1;
        h[12] = 90;
        h[13] = 240;
        h[15] = 252;
        h[16] = 253;
        h[17] = 248;
        h[18] = 248;
        h[21] = 3;
        h[22] = 1;
        h.extend_from_slice(&[0xa0, 0, 1, 0, 3, 64, 1, 128]);
        let c = HevcConfig::parse(&h).unwrap();
        assert_eq!(c.arrays[0].nal_type, 32);
        assert_eq!(c.length_size, 4);
        assert_eq!(c.bit_depth_luma, 8);
        assert_eq!(c.chroma_format, 1);
        for n in 0..h.len() {
            assert!(HevcConfig::parse(&h[..n]).is_err());
        }
        h[30] = 0; // payload is not validated by a configuration parser.
        h[29] = 0;
        assert!(HevcConfig::parse(&h).is_err());
    }
    #[test]
    fn nal_framing_terminates_on_error() {
        for size in [1, 2, 4] {
            let mut packet = vec![0; size as usize - 1];
            packet.extend_from_slice(&[2, 0x65, 128]);
            let mut units = NalUnits::new(&packet, size).unwrap();
            assert_eq!(units.next().unwrap().unwrap(), &[0x65, 128]);
            assert!(units.next().is_none());
            packet.pop();
            let mut units = NalUnits::new(&packet, size).unwrap();
            assert!(units.next().unwrap().is_err());
            assert!(units.next().is_none());
        }
        assert!(NalUnits::new(&[], 3).is_err());
    }
    #[test]
    fn aac_lc_and_sbr_absent_extension() {
        let c = AacConfig::parse(&[0x12, 0x10]).unwrap();
        assert_eq!(
            (c.sample_rate, c.channels, c.frame_samples),
            (44100, 2, 1024)
        );
        // 48 kHz mono LC + syncExtensionType 0x2b7, AOT 5, sbrPresentFlag=0.
        let c = AacConfig::parse(&[0x11, 0x88, 0x56, 0xe5, 0]).unwrap();
        assert_eq!((c.sample_rate, c.channels), (48000, 1));
        for bytes in [
            &[][..],
            &[0x12],
            &[0x12, 0],
            &[0x12, 0x11],
            &[0x11, 0x88, 0x56, 0xe5, 0x80],
        ] {
            assert!(AacConfig::parse(bytes).is_err());
        }
    }
    #[test]
    fn esds_extracts_only_nested_decoder_specific_data() {
        let dc = [
            0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 2, 0x12, 0x10,
        ];
        let mut esds = vec![0, 0, 0, 0, 3, 22, 0, 1, 0, 4, 17];
        esds.extend_from_slice(&dc);
        assert_eq!(aac_specific_config(&esds).unwrap(), &[0x12, 0x10]);
        for n in 0..esds.len() {
            assert!(aac_specific_config(&esds[..n]).is_err());
        }
        esds[11] = 0x6b;
        assert!(aac_specific_config(&esds).is_err());
    }
}
