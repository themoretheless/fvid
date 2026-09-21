//! AV1 low-overhead OBU framing. Parsing is not pixel decoding.
//! Reference: AOM AV1 specification, section 5.3.
use crate::{Result, invalid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Obu<'a> {
    pub kind: u8,
    pub temporal_id: u8,
    pub spatial_id: u8,
    pub payload: &'a [u8],
}

/// A bounded, zero-copy iterator over low-overhead OBUs. An error terminates it.
/// Length-delimited Annex B and RTP aggregation are different framing formats.
pub struct Obus<'a> {
    remaining: &'a [u8],
}
impl<'a> Obus<'a> {
    pub fn new(packet: &'a [u8]) -> Self {
        Self { remaining: packet }
    }
}

fn leb128(bytes: &[u8]) -> Result<(usize, usize)> {
    let mut value = 0u64;
    for i in 0..8 {
        let b = *bytes
            .get(i)
            .ok_or_else(|| invalid("truncated AV1 LEB128"))?;
        value |= u64::from(b & 127) << (7 * i);
        if b & 128 == 0 {
            // AV1 syntax limits leb128 values to 32 unsigned bits.
            if value > u64::from(u32::MAX) {
                return Err(invalid("AV1 LEB128 exceeds 32 bits"));
            }
            return Ok((value as usize, i + 1));
        }
    }
    Err(invalid("unterminated AV1 LEB128"))
}

fn read_obu(packet: &[u8]) -> Result<(Obu<'_>, &[u8])> {
    let h = *packet
        .first()
        .ok_or_else(|| invalid("missing AV1 OBU header"))?;
    if h & 0x81 != 0 {
        return Err(invalid("invalid AV1 OBU reserved/forbidden bits"));
    }
    if h & 2 == 0 {
        return Err(invalid("AV1 low-overhead OBU requires a size field"));
    }
    let mut offset = 1;
    let mut temporal_id = 0;
    let mut spatial_id = 0;
    if h & 4 != 0 {
        let e = *packet
            .get(offset)
            .ok_or_else(|| invalid("truncated AV1 OBU extension"))?;
        if e & 7 != 0 {
            return Err(invalid("invalid AV1 OBU extension reserved bits"));
        }
        temporal_id = e >> 5;
        spatial_id = (e >> 3) & 3;
        offset += 1;
    }
    let (length, consumed) = leb128(&packet[offset..])?;
    offset += consumed;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid("AV1 OBU size overflow"))?;
    let payload = packet
        .get(offset..end)
        .ok_or_else(|| invalid("AV1 OBU exceeds packet"))?;
    let kind = (h >> 3) & 15;
    if kind == 2 && !payload.is_empty() {
        return Err(invalid("AV1 temporal delimiter must be empty"));
    }
    Ok((
        Obu {
            kind,
            temporal_id,
            spatial_id,
            payload,
        },
        &packet[end..],
    ))
}
impl<'a> Iterator for Obus<'a> {
    type Item = Result<Obu<'a>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        match read_obu(self.remaining) {
            Ok((obu, tail)) => {
                self.remaining = tail;
                Some(Ok(obu))
            }
            Err(error) => {
                self.remaining = &[];
                Some(Err(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layers_reserved_units_and_nonminimal_lengths() {
        let packet = [0x12, 0, 0x7e, 0x68, 0x82, 0, 1, 2];
        let obus = Obus::new(&packet).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(obus.len(), 2);
        assert_eq!(
            obus[1],
            Obu {
                kind: 15,
                temporal_id: 3,
                spatial_id: 1,
                payload: &[1, 2]
            }
        );
    }
    #[test]
    fn invalid_lengths_and_headers_terminate() {
        for packet in [
            &[0x80][..],
            &[0x10],
            &[0x12, 1, 0],
            &[0x0e],
            &[0x0e, 1, 0],
            &[0x0a, 0x80],
            &[0x0a, 4, 1],
            &[0x0a, 255, 255, 255, 255, 16],
            &[0x0a, 128, 128, 128, 128, 128, 128, 128, 128],
        ] {
            let mut obus = Obus::new(packet);
            assert!(obus.next().unwrap().is_err(), "{packet:?}");
            assert!(obus.next().is_none());
        }
    }
    #[test]
    fn every_truncation_is_bounded() {
        let packet = [0x0e, 0, 4, 12, 34, 56, 78];
        for end in 1..packet.len() {
            assert!(Obus::new(&packet[..end]).next().unwrap().is_err());
        }
        assert_eq!(
            Obus::new(&packet).next().unwrap().unwrap().payload,
            &[12, 34, 56, 78]
        );
    }
}
