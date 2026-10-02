const MDCV_PAYLOAD_LEN: usize = 24;
const CLLI_PAYLOAD_LEN: usize = 4;
fn chroma(raw: u32) -> f64 { f64::from(raw) * 0.00002 }
fn luminance(raw: u32) -> f32 { raw as f32 * 0.0001 }
fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}


impl MasteringDisplay {
    /// Decode a mastering display colour volume payload.
    pub fn from_payload(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < MDCV_PAYLOAD_LEN {
            return None;
        }
        let mut pairs = [(0u16, 0u16); 4];
        for (slot, pair) in pairs.iter_mut().enumerate() {
            *pair = (
                be16(&bytes[slot * 4..slot * 4 + 2]),
                be16(&bytes[slot * 4 + 2..slot * 4 + 4]),
            );
        }
        let point = |p: (u16, u16)| (chroma(u32::from(p.0)), chroma(u32::from(p.1)));
        // The corners are carried in the standard's green, blue, red order and
        // placed into the corners they name.
        Self::from_corners(
            point(pairs[2]),
            point(pairs[0]),
            point(pairs[1]),
            point(pairs[3]),
            luminance(be32(&bytes[16..20])),
            luminance(be32(&bytes[20..24])),
        )
    }

}
impl HdrMetadata {
    /// Metadata from a content light level payload.
    pub fn from_clli(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < CLLI_PAYLOAD_LEN {
            return None;
        }
        Some(Self {
            mastering: None,
            light: ContentLight {
                max_cll: f32::from(be16(&bytes[..2])),
                max_fall: f32::from(be16(&bytes[2..])),
            },
        })
    }

    /// Metadata from a mastering display payload.
    pub fn from_mdcv(bytes: &[u8]) -> Option<Self> {
        Some(Self {
            mastering: MasteringDisplay::from_payload(bytes),
            light: ContentLight::default(),
        })
    }

    /// Take whichever fields the newer block actually carries.
    ///
    /// A stream can signal the two halves in different places — an MP4 `mdcv`
    /// box next to an in-band SEI 144, for instance — so a block that only
    /// knows one of them must not clear the other.
    pub fn merge(&mut self, other: Self) {
        if other.mastering.is_some() {
            self.mastering = other.mastering;
        }
        if other.light.max_cll > 0.0 || other.light.max_fall > 0.0 {
            self.light = other.light;
        }
    }

}
