//! Transactional per-elementary-stream SL AU reconstruction.
use super::{Config, PacketKind, Result, invalid};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessUnit {
    pub data: Vec<u8>,
    pub bit_len: usize,
    pub sequence: Option<u16>,
    pub random_access: bool,
    pub decoding_timestamp: Option<u64>,
    pub composition_timestamp: Option<u64>,
}
#[derive(Debug)]
pub struct Push {
    /// Up to two AUs, in source order, when a new start resolves a prior end.
    pub access_units: Vec<AccessUnit>,
    pub duplicate_packet: bool,
    /// An immediately repeated packet may carry a changed OCR.
    pub ocr: Option<u64>,
    pub degradation_priority: Option<u16>,
    pub instant_bitrate: Option<Bitrate>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bitrate {
    pub data: [u8; 32],
    pub bit_len: u8,
}
fn bitrate(field: Option<super::BitField<'_>>) -> Option<Bitrate> {
    field.map(|field| {
        let mut value = Bitrate {
            data: [0; 32],
            bit_len: field.bit_len,
        };
        for i in 0..field.bit_len {
            if field.bit(i) == Some(true) {
                value.data[usize::from(i) / 8] |= 1 << (7 - i % 8);
            }
        }
        value
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    unit: AccessUnit,
    length: Option<u32>,
    end_unknown: bool,
    discard: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    pending: Option<Pending>,
    finished: bool,
    previous_end: Option<bool>,
    next_packet_sequence: Option<u16>,
    last_packet_sequence: Option<u16>,
    last_packet: Vec<u8>,
    last_ocr: Option<(usize, u8)>,
    last_previous_end: Option<bool>,
    last_au_sequence: Option<u16>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            pending: None,
            finished: false,
            previous_end: Some(true),
            next_packet_sequence: None,
            last_packet_sequence: None,
            last_packet: Vec::new(),
            last_ocr: None,
            last_previous_end: None,
            last_au_sequence: None,
        }
    }
}
/// One elementary stream, beginning at a known stream/AU boundary.
/// Transport packet lengths and stream routing are supplied by the caller.
pub struct Assembler {
    config: Config,
    max_au_bytes: Option<usize>,
    state: State,
}
#[derive(Clone)]
pub struct Checkpoint {
    config: Config,
    max_au_bytes: Option<usize>,
    state: State,
}
fn next(value: u16, width: u8) -> u16 {
    ((u32::from(value) + 1) & ((1u32 << width) - 1)) as u16
}
fn valid_end(p: &Pending) -> Result<()> {
    if p.length
        .is_some_and(|n| p.unit.bit_len.div_ceil(8) != n as usize)
    {
        return Err(invalid("SL AU end disagrees with declared length"));
    }
    Ok(())
}
fn same_packet(old: &[u8], new: &[u8], ocr: Option<(usize, u8)>) -> bool {
    if old.len() != new.len() {
        return false;
    }
    for (i, (&a, &b)) in old.iter().zip(new).enumerate() {
        let mut mask = 0u8;
        if let Some((start, length)) = ocr {
            let end = start + usize::from(length);
            let first = if i >= start / 8 && i < end.div_ceil(8) {
                start.max(i * 8)
            } else {
                0
            };
            let end = if i >= start / 8 && i < end.div_ceil(8) {
                end.min(i * 8 + 8)
            } else {
                0
            };
            for bit in first..end {
                mask |= 1 << (7 - bit % 8);
            }
        }
        if a & !mask != b & !mask {
            return false;
        }
    }
    true
}
fn append(target: &mut Vec<u8>, old_bits: usize, data: &[u8], bits: usize) {
    let total = old_bits + bits;
    target.resize(total.div_ceil(8), 0);
    if bits == 0 {
        return;
    }
    let base = old_bits / 8;
    let shift = old_bits % 8;
    let count = bits.div_ceil(8);
    if shift == 0 {
        target[base..base + count].copy_from_slice(&data[..count]);
    } else {
        for (i, &value) in data[..count].iter().enumerate() {
            target[base + i] |= value >> shift;
            if base + i + 1 < target.len() {
                target[base + i + 1] |= value << (8 - shift);
            }
        }
    }
    if total % 8 != 0 {
        *target.last_mut().unwrap() &= 0xff << (8 - total % 8);
    }
}
impl Assembler {
    pub fn new(config: Config) -> Result<Self> {
        Self::with_max_au_bytes(config, None)
    }
    /// Optional caller policy; there is no implicit global media-size limit.
    pub fn with_max_au_bytes(config: Config, max_au_bytes: Option<usize>) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            max_au_bytes,
            state: State::default(),
        })
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            config: self.config.clone(),
            max_au_bytes: self.max_au_bytes,
            state: self.state.clone(),
        }
    }
    pub fn restore(&mut self, checkpoint: &Checkpoint) -> Result<()> {
        if self.config != checkpoint.config || self.max_au_bytes != checkpoint.max_au_bytes {
            return Err(invalid("SL checkpoint configuration mismatch"));
        }
        self.state = checkpoint.state.clone();
        Ok(())
    }
    pub fn reset(&mut self) {
        self.state = State::default();
    }
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        self.state
            .last_packet
            .capacity()
            .checked_add(
                self.state
                    .pending
                    .as_ref()
                    .map_or(0, |p| p.unit.data.capacity()),
            )
            .ok_or_else(|| invalid("SL retained payload size overflow"))
    }
    pub fn push(&mut self, data: &[u8]) -> Result<Push> {
        if self.state.finished {
            return Err(invalid("SL packet after end of stream"));
        }
        // A repeated end-only packet must retain its original implicit start.
        // Parsing against the already advanced previous_end changes its header.
        if self.state.last_packet_sequence.is_some()
            && same_packet(&self.state.last_packet, data, self.state.last_ocr)
        {
            let p = self
                .config
                .parse_packet(data, self.state.last_previous_end)?;
            return Ok(Push {
                access_units: Vec::new(),
                duplicate_packet: true,
                ocr: p.ocr,
                degradation_priority: p.degradation_priority,
                instant_bitrate: bitrate(p.instant_bitrate),
            });
        }
        let p = self.config.parse_packet(data, self.state.previous_end)?;
        let ocr_span = p.ocr_field.map(|field| (field.start, field.bit_len));
        if let Some(sequence) = p.packet_sequence {
            if self.state.last_packet_sequence == Some(sequence) {
                if ocr_span != self.state.last_ocr
                    || !same_packet(&self.state.last_packet, data, ocr_span)
                {
                    return Err(invalid("nonidentical duplicate SL packet"));
                }
                return Ok(Push {
                    access_units: Vec::new(),
                    duplicate_packet: true,
                    ocr: p.ocr,
                    degradation_priority: p.degradation_priority,
                    instant_bitrate: bitrate(p.instant_bitrate),
                });
            }
            if self
                .state
                .next_packet_sequence
                .is_some_and(|expected| sequence != expected)
            {
                return Err(invalid("SL packet sequence discontinuity"));
            }
        }
        if p.kind != PacketKind::Data {
            if let Some(expected) = self.state.next_packet_sequence {
                self.state.next_packet_sequence =
                    Some(next(expected, self.config.packet_sequence_length));
            }
            self.state.previous_end = p.access_unit_end;
            self.state.last_packet_sequence = None;
            self.state.last_packet = Vec::new();
            self.state.last_ocr = None;
            return Ok(Push {
                access_units: Vec::new(),
                duplicate_packet: false,
                ocr: p.ocr,
                degradation_priority: p.degradation_priority,
                instant_bitrate: bitrate(p.instant_bitrate),
            });
        }
        let start = p.access_unit_start;
        if start {
            if let Some(pending) = &self.state.pending {
                if !pending.end_unknown {
                    return Err(invalid("SL new AU starts before preceding AU ended"));
                }
                valid_end(pending)?;
            }
        } else if self.state.pending.is_none() && p.payload_bits != 0 {
            return Err(invalid("SL continuation has no preceding AU"));
        }
        let previous_au = self
            .state
            .pending
            .as_ref()
            .and_then(|p| p.unit.sequence)
            .or(self.state.last_au_sequence);
        let mut discard = false;
        if start {
            if let (Some(previous), Some(sequence)) = (previous_au, p.au_sequence) {
                if sequence == previous {
                    discard = p.random_access;
                } else if sequence != next(previous, self.config.au_sequence_length) {
                    return Err(invalid("SL AU sequence discontinuity"));
                }
            }
        }
        let old_bits = if start {
            0
        } else {
            self.state.pending.as_ref().map_or(0, |p| p.unit.bit_len)
        };
        let total = old_bits
            .checked_add(p.payload_bits)
            .ok_or_else(|| invalid("SL AU bit length overflow"))?;
        let length = if start {
            p.access_unit_length
        } else {
            self.state.pending.as_ref().and_then(|p| p.length)
        };
        if self
            .max_au_bytes
            .is_some_and(|limit| total.div_ceil(8) > limit)
        {
            return Err(invalid("SL AU exceeds caller limit"));
        }
        if length.is_some_and(|size| total.div_ceil(8) > size as usize) {
            return Err(invalid("SL AU exceeds declared length"));
        }
        let finish = p.access_unit_end == Some(true)
            || (p.access_unit_end.is_none()
                && length.is_some_and(|size| total % 8 == 0 && total / 8 == size as usize));
        if finish && length.is_some_and(|size| total.div_ceil(8) != size as usize) {
            return Err(invalid("SL AU end disagrees with declared length"));
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(2)
            .map_err(|_| invalid("SL output allocation failed"))?;
        let mut cached = Vec::new();
        if p.packet_sequence.is_some() {
            cached
                .try_reserve_exact(data.len())
                .map_err(|_| invalid("SL packet cache allocation failed"))?;
            cached.extend_from_slice(data);
        }
        let mut fresh = if start {
            Some(Pending {
                unit: AccessUnit {
                    data: Vec::new(),
                    bit_len: 0,
                    sequence: p.au_sequence,
                    random_access: p.random_access,
                    decoding_timestamp: p.decoding_timestamp,
                    composition_timestamp: p.composition_timestamp,
                },
                length,
                end_unknown: p.access_unit_end.is_none(),
                discard,
            })
        } else {
            None
        };
        // Allocation is the last fallible operation. No parser, sequence or
        // length error may occur after any logical state has changed.
        if let Some(pending) = fresh.as_mut().or(self.state.pending.as_mut()) {
            if !pending.discard {
                pending
                    .unit
                    .data
                    .try_reserve(total.div_ceil(8).saturating_sub(pending.unit.data.len()))
                    .map_err(|_| invalid("SL AU allocation failed"))?;
            }
        }
        if start {
            if let Some(old) = self.state.pending.take() {
                self.state.last_au_sequence = old.unit.sequence;
                if !old.discard {
                    output.push(old.unit);
                }
            }
            self.state.pending = fresh;
        }
        if let Some(pending) = &mut self.state.pending {
            if !pending.discard {
                append(&mut pending.unit.data, old_bits, p.payload, p.payload_bits);
            }
            pending.unit.bit_len = total;
            pending.end_unknown = p.access_unit_end.is_none();
            if finish {
                let complete = self.state.pending.take().unwrap();
                self.state.last_au_sequence = complete.unit.sequence;
                if !complete.discard {
                    output.push(complete.unit);
                }
            }
        }
        self.state.last_previous_end = self.state.previous_end;
        self.state.previous_end = p.access_unit_end;
        if let Some(sequence) = p.packet_sequence {
            self.state.next_packet_sequence =
                Some(next(sequence, self.config.packet_sequence_length));
        }
        self.state.last_packet_sequence = p.packet_sequence;
        self.state.last_packet = cached;
        self.state.last_ocr = ocr_span;
        Ok(Push {
            access_units: output,
            duplicate_packet: false,
            ocr: p.ocr,
            degradation_priority: p.degradation_priority,
            instant_bitrate: bitrate(p.instant_bitrate),
        })
    }
    /// Signal the lower layer's clean end of stream. An omitted end flag is
    /// resolved by that boundary; an explicit unfinished AU is still an error.
    /// Callers must not use this to turn a truncated transport into clean EOF.
    pub fn finish(&mut self) -> Result<Option<AccessUnit>> {
        if self.state.finished {
            return Ok(None);
        }
        if let Some(pending) = &self.state.pending {
            if !pending.end_unknown {
                return Err(invalid("SL EOF before explicit AU end"));
            }
            valid_end(pending)?;
        }
        let complete = self.state.pending.take();
        self.state.finished = true;
        self.state.last_packet = Vec::new();
        self.state.last_packet_sequence = None;
        self.state.last_ocr = None;
        if let Some(complete) = complete {
            self.state.last_au_sequence = complete.unit.sequence;
            Ok(if complete.discard {
                None
            } else {
                Some(complete.unit)
            })
        } else {
            Ok(None)
        }
    }
}
