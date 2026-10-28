//! Owned MPEG-4 Systems base SL configuration and bounded packet syntax.
//! ISO/IEC 14496-1:2010 7.3.2.3–7.3.2.4. External framing supplies packet size.
use crate::owned_aac::{Error, Result, bits::BitReader};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duration {
    pub timescale: u32,
    pub access_unit: u16,
    pub composition_unit: u16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub predefined: u8,
    pub use_start: bool,
    pub use_end: bool,
    pub use_random_access: bool,
    pub random_access_only: bool,
    pub use_padding: bool,
    pub use_timestamps: bool,
    pub use_idle: bool,
    pub timestamp_resolution: u32,
    pub ocr_resolution: u32,
    pub timestamp_length: u8,
    pub ocr_length: u8,
    pub au_length: u8,
    pub bitrate_length: u8,
    pub priority_length: u8,
    pub au_sequence_length: u8,
    pub packet_sequence_length: u8,
    pub duration: Option<Duration>,
    pub start_timestamps: Option<(u64, u64)>,
}
fn word(bits: &mut BitReader<'_>, length: u8) -> Result<u64> {
    if length > 64 {
        return Err(invalid("SL integer field exceeds 64 bits"));
    }
    if length <= 32 {
        return Ok(u64::from(bits.read(length)?));
    }
    Ok((u64::from(bits.read(length - 32)?) << 32) | u64::from(bits.read(32)?))
}
impl Config {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut b = BitReader::new(data);
        let predefined = b.read(8)? as u8;
        let mut c = Self {
            predefined,
            use_start: false,
            use_end: false,
            use_random_access: false,
            random_access_only: false,
            use_padding: false,
            use_timestamps: predefined == 2,
            use_idle: false,
            timestamp_resolution: if predefined == 1 { 1000 } else { 0 },
            ocr_resolution: 0,
            timestamp_length: if predefined == 1 { 32 } else { 0 },
            ocr_length: 0,
            au_length: 0,
            bitrate_length: 0,
            priority_length: 0,
            au_sequence_length: 0,
            packet_sequence_length: 0,
            duration: None,
            start_timestamps: None,
        };
        if predefined == 0 {
            c.use_start = b.bit()?;
            c.use_end = b.bit()?;
            c.use_random_access = b.bit()?;
            c.random_access_only = b.bit()?;
            c.use_padding = b.bit()?;
            c.use_timestamps = b.bit()?;
            c.use_idle = b.bit()?;
            let duration = b.bit()?;
            c.timestamp_resolution = b.read(32)?;
            c.ocr_resolution = b.read(32)?;
            c.timestamp_length = b.read(8)? as u8;
            c.ocr_length = b.read(8)? as u8;
            c.au_length = b.read(8)? as u8;
            c.bitrate_length = b.read(8)? as u8;
            c.priority_length = b.read(4)? as u8;
            c.au_sequence_length = b.read(5)? as u8;
            c.packet_sequence_length = b.read(5)? as u8;
            if b.read(2)? != 3 {
                return Err(invalid("invalid SL configuration reserved bits"));
            }
            if duration {
                c.duration = Some(Duration {
                    timescale: b.read(32)?,
                    access_unit: b.read(16)? as u16,
                    composition_unit: b.read(16)? as u16,
                });
            }
        } else if predefined > 2 {
            return Err(invalid("reserved predefined SL configuration"));
        }
        c.validate()?;
        if !c.use_timestamps {
            c.start_timestamps = Some((
                word(&mut b, c.timestamp_length)?,
                word(&mut b, c.timestamp_length)?,
            ));
        }
        // Final descriptor alignment is not an additional descriptor payload.
        if b.remaining() > 7 {
            return Err(invalid("extra SL configuration bytes"));
        }
        Ok(c)
    }
    fn validate(&self) -> Result<()> {
        if self.predefined > 2
            || self.timestamp_length > 64
            || self.ocr_length > 64
            || self.au_length > 32
            || self.priority_length > 15
            || self.au_sequence_length > 16
            || self.packet_sequence_length > 16
        {
            return Err(invalid("invalid SL configuration field width"));
        }
        if self.timestamp_length > 0 && self.timestamp_resolution == 0 {
            return Err(invalid("zero SL timestamp resolution"));
        }
        if self.ocr_length > 0 && self.ocr_resolution == 0 {
            return Err(invalid("zero SL OCR resolution"));
        }
        if self.duration.as_ref().is_some_and(|d| d.timescale == 0) {
            return Err(invalid("zero SL duration timescale"));
        }
        Ok(())
    }
    /// Bind a bare MP4 sample using the container's exact valid-bit count.
    /// No SL header is removed and no timestamp is inferred from sample bytes.
    pub fn mp4_sample<'a>(&self, data: &'a [u8], bit_len: usize) -> Result<Payload<'a>> {
        self.validate()?;
        if self.predefined != 2 {
            return Err(invalid(
                "MP4 sample binding requires predefined SL configuration 2",
            ));
        }
        if data.len() != bit_len.div_ceil(8) {
            return Err(invalid("MP4 SL sample bit boundary disagrees with storage"));
        }
        Ok(Payload { data, bit_len })
    }
    /// Parse one externally framed SL packet, not a bare MP4 media sample.
    /// When only end flags are carried,
    /// previous_end supplies the implicit start decision. An absent end flag
    /// remains None until the next packet or AU-length assembler resolves it.
    pub fn parse_packet<'a>(
        &self,
        data: &'a [u8],
        previous_end: Option<bool>,
    ) -> Result<Packet<'a>> {
        self.validate()?;
        let mut b = BitReader::new(data);
        let explicit_start = if self.use_start { Some(b.bit()?) } else { None };
        let explicit_end = if self.use_end { Some(b.bit()?) } else { None };
        let ocr_flag = self.ocr_length > 0 && b.bit()?;
        let idle = self.use_idle && b.bit()?;
        let padding = self.use_padding && b.bit()?;
        let padding_bits = if padding { b.read(3)? as u8 } else { 0 };
        let kind = if idle {
            PacketKind::Idle
        } else if padding && padding_bits == 0 {
            PacketKind::PaddingOnly
        } else {
            PacketKind::Data
        };
        if padding && padding_bits == 0 && (explicit_start == Some(true) || ocr_flag) {
            return Err(invalid("SL padding-only packet carries AU start or OCR"));
        }
        let active = kind == PacketKind::Data;
        let start = if !active {
            explicit_start.unwrap_or(false)
        } else if let Some(start) = explicit_start {
            start
        } else if !self.use_end {
            true
        } else {
            previous_end.ok_or_else(|| invalid("SL AU start requires preceding packet context"))?
        };
        let end = explicit_end.or_else(|| {
            if !self.use_start && !self.use_end && active {
                Some(true)
            } else {
                None
            }
        });
        let mut p = Packet {
            kind,
            access_unit_start: start,
            access_unit_end: end,
            random_access: false,
            packet_sequence: None,
            au_sequence: None,
            degradation_priority: None,
            ocr: None,
            decoding_timestamp: None,
            composition_timestamp: None,
            access_unit_length: None,
            instant_bitrate: None,
            header_bytes: 0,
            payload: &[],
            payload_bits: 0,
        };
        if active {
            if self.packet_sequence_length > 0 {
                p.packet_sequence = Some(b.read(self.packet_sequence_length)? as u16);
            }
            if self.priority_length > 0 && b.bit()? {
                p.degradation_priority = Some(b.read(self.priority_length)? as u16);
            }
            if ocr_flag {
                p.ocr = Some(word(&mut b, self.ocr_length)?);
            }
            if start {
                p.random_access = if self.use_random_access {
                    b.bit()?
                } else {
                    self.random_access_only
                };
                if self.au_sequence_length > 0 {
                    p.au_sequence = Some(b.read(self.au_sequence_length)? as u16);
                }
                let dts = self.use_timestamps && b.bit()?;
                let cts = self.use_timestamps && b.bit()?;
                let bitrate = self.bitrate_length > 0 && b.bit()?;
                if dts {
                    p.decoding_timestamp = Some(word(&mut b, self.timestamp_length)?);
                }
                if cts {
                    p.composition_timestamp = Some(word(&mut b, self.timestamp_length)?);
                }
                if self.au_length > 0 {
                    let size = b.read(self.au_length)?;
                    if size != 0 {
                        p.access_unit_length = Some(size);
                    }
                }
                if bitrate {
                    let start = b.position();
                    b.skip(usize::from(self.bitrate_length))?;
                    p.instant_bitrate = Some(BitField {
                        data,
                        start,
                        bit_len: self.bitrate_length,
                    });
                }
            }
        }
        b.skip((8 - b.position() % 8) % 8)?;
        p.header_bytes = b.position() / 8;
        if active {
            p.payload = &data[p.header_bytes..];
            let total = p
                .payload
                .len()
                .checked_mul(8)
                .ok_or_else(|| invalid("SL payload length overflow"))?;
            p.payload_bits = total
                .checked_sub(usize::from(padding_bits))
                .ok_or_else(|| invalid("SL padding exceeds packet payload"))?;
            if padding_bits != 0
                && p.payload
                    .last()
                    .is_none_or(|v| v & ((1 << padding_bits) - 1) != 0)
            {
                return Err(invalid("nonzero SL payload padding bits"));
            }
            if let Some(size) = p.access_unit_length {
                let size = usize::try_from(size)
                    .map_err(|_| invalid("SL AU length exceeds address space"))?;
                if p.payload.len() > size || (start && end == Some(true) && p.payload.len() != size)
                {
                    return Err(invalid("SL packet payload disagrees with AU length"));
                }
            }
        }
        Ok(p)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketKind {
    Data,
    Idle,
    PaddingOnly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitField<'a> {
    data: &'a [u8],
    pub start: usize,
    pub bit_len: u8,
}
impl BitField<'_> {
    pub fn bit(&self, index: u8) -> Option<bool> {
        if index >= self.bit_len {
            return None;
        }
        let at = self.start.checked_add(usize::from(index))?;
        Some(self.data.get(at / 8)? >> (7 - at % 8) & 1 != 0)
    }
    pub fn as_u64(&self) -> Option<u64> {
        if self.bit_len > 64 {
            return None;
        }
        let mut b = BitReader::new(self.data);
        b.skip(self.start).ok()?;
        word(&mut b, self.bit_len).ok()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    pub kind: PacketKind,
    pub access_unit_start: bool,
    pub access_unit_end: Option<bool>,
    pub random_access: bool,
    pub packet_sequence: Option<u16>,
    pub au_sequence: Option<u16>,
    pub degradation_priority: Option<u16>,
    pub ocr: Option<u64>,
    pub decoding_timestamp: Option<u64>,
    pub composition_timestamp: Option<u64>,
    /// Bytes; zero in the bitstream means unknown and is represented by None.
    pub access_unit_length: Option<u32>,
    pub instant_bitrate: Option<BitField<'a>>,
    pub header_bytes: usize,
    pub payload: &'a [u8],
    pub payload_bits: usize,
}

/// A bounded payload; unused low storage bits are outside the access unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Payload<'a> {
    pub data: &'a [u8],
    pub bit_len: usize,
}
