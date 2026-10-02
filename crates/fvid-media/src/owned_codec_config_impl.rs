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
            if !matches!(profile, 100 | 110 | 122 | 144 | 244) {
                return Err(invalid("unexpected AVC profile extension"));
            }
            // Some writers mis-state reserved bits (and format hints) here.
            // Decoders derive chroma/depth from SPS, not these redundant hints.
            // Still require the entire extension and validate each NAL/length.
            b.take(3)?;
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
                let parsed = hevc_nal::NalHeader::parse(nal)?;
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
        // Some MP4 writers clear the reserved low bit (0x14 instead of
        // 0x15). It does not change the object or audio stream type; ASC
        // validation below still determines the supported AAC configuration.
        if header[0] != 0x40 || header[1] >> 2 != 5 {
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
