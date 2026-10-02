#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NalHeader {
    pub unit_type: u8,
    pub layer_id: u8,
    pub temporal_id: u8,
}
impl NalHeader {
    /// Parse header syntax without interpreting reserved NAL types as known data.
    /// Multilayer headers remain representable; a base-layer decoder must reject
    /// unsupported layer IDs explicitly before processing their payloads.
    pub fn parse(nal: &[u8]) -> Result<Self> {
        let header = nal
            .get(..2)
            .ok_or_else(|| invalid("truncated HEVC NAL header"))?;
        if header[0] & 0x80 != 0 || header[1] & 7 == 0 {
            return Err(invalid("invalid HEVC forbidden/temporal header bits"));
        }
        Ok(Self {
            unit_type: (header[0] >> 1) & 63,
            layer_id: ((header[0] & 1) << 5) | (header[1] >> 3),
            temporal_id: (header[1] & 7) - 1,
        })
    }
    pub fn is_vcl(self) -> bool {
        self.unit_type < 32
    }
    pub fn is_irap(self) -> bool {
        (16..=23).contains(&self.unit_type)
    }
    pub fn is_idr(self) -> bool {
        matches!(self.unit_type, 19 | 20)
    }
    pub fn require_base_layer(self) -> Result<()> {
        if self.layer_id != 0 {
            return Err(invalid("HEVC multilayer decoding is not implemented"));
        }
        Ok(())
    }
}

pub struct NalRbsp {
    pub header: NalHeader,
    pub bytes: Vec<u8>,
}
impl NalRbsp {
    /// Budget bounds the allocation before de-escaping untrusted input.
    /// RBSP trailing bits depend on payload syntax and are checked by its reader.
    pub fn parse(nal: &[u8], budget: usize) -> Result<Self> {
        let header = NalHeader::parse(nal)?;
        let payload = &nal[2..];
        if payload.is_empty() || payload.len() > budget {
            return Err(invalid("empty HEVC payload or RBSP budget exceeded"));
        }
        Ok(Self {
            header,
            bytes: unescape_rbsp(payload)?,
        })
    }
}
