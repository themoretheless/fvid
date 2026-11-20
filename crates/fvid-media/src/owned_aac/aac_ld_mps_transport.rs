//! AAC ER extension envelope and MPEG Surround ancillary-fragment assembly.
//! Payload bytes stay opaque here; config/frame parsing and PCM are downstream.
use super::{bits::BitReader, invalid, Result};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extension {
    LowDelay,
    Surround,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Frame,
    HeaderAndFrame,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub extension: Extension,
    pub kind: Kind,
    pub payload: Vec<u8>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assembler {
    pending: Option<Vec<u8>>,
    replacements: u64,
    prefix_losses: u64,
}
impl Assembler {
    /// End-of-stream validation does not discard an unfinished message.
    pub fn finish(&self) -> Result<()> {
        if self.pending.is_some() {
            Err(invalid("unfinished MPEG Surround fragmented message"))
        } else {
            Ok(())
        }
    }
    pub fn reset(&mut self) {
        self.pending = None;
        self.replacements = 0;
        self.prefix_losses = 0;
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    /// Completed messages which replaced an unfinished ancillary sequence.
    pub fn replacements(&self) -> u64 {
        self.replacements
    }
    /// Replacements that do not preserve the complete accumulated prefix.
    /// A full retransmission starting with that prefix assembles exactly the
    /// same message as a stop fragment carrying only its remaining suffix.
    pub fn prefix_losses(&self) -> u64 {
        self.prefix_losses
    }
    /// `end` is the absolute bound supplied by the AAC envelope owner. Direct
    /// types9/12 consume that bound; type3 consumes only its length-delimited
    /// child, preserving following extensions. No implicit alignment is applied.
    /// Errors retain both pending fragments and the reader position.
    pub fn read(&mut self, bits: &mut BitReader<'_>, end: usize) -> Result<Option<Message>> {
        let mut reader = bits.clone();
        let available = reader
            .position()
            .checked_add(reader.remaining())
            .ok_or_else(|| invalid("AAC spatial extension bound overflow"))?;
        if end > available || end < reader.position() + 4 {
            return Err(invalid("truncated AAC spatial extension tag"));
        }
        let bounded = |r: &mut BitReader<'_>, n: u8, limit: usize| -> Result<u32> {
            if usize::from(n) > limit.saturating_sub(r.position()) {
                return Err(invalid("truncated AAC spatial extension envelope"));
            }
            r.read(n)
        };
        let tag = bounded(&mut reader, 4, end)?;
        let (tag, limit) = if tag == 3 {
            let mut length = bounded(&mut reader, 4, end)? as usize;
            if length == 15 {
                let add = bounded(&mut reader, 8, end)? as usize;
                length += add;
                if add == 255 {
                    length += bounded(&mut reader, 16, end)? as usize;
                }
            }
            let child_end = reader
                .position()
                .checked_add(
                    length
                        .checked_mul(8)
                        .ok_or_else(|| invalid("AAC spatial child length overflow"))?,
                )
                .ok_or_else(|| invalid("AAC spatial child bound overflow"))?;
            if child_end > end {
                return Err(invalid("AAC spatial child exceeds parent extension"));
            }
            let child = bounded(&mut reader, 4, child_end)?;
            if child == 3 {
                return Err(invalid("nested AAC EXT_DATA_LENGTH is forbidden"));
            }
            (child, child_end)
        } else {
            (tag, end)
        };
        let extension = match tag {
            9 => Extension::LowDelay,
            12 => Extension::Surround,
            _ => return Err(invalid("expected AAC LD-MPS/spatial extension tag")),
        };
        let kind = match bounded(&mut reader, 2, limit)? {
            0 => Kind::Frame,
            1 => Kind::HeaderAndFrame,
            _ => return Err(invalid("reserved MPEG Surround ancillary type")),
        };
        let segment = bounded(&mut reader, 2, limit)?;
        let remaining = limit - reader.position();
        if remaining % 8 != 0 {
            return Err(invalid("non-byte MPEG Surround fragment body"));
        }
        let mut payload = Vec::new();
        payload
            .try_reserve(remaining / 8)
            .map_err(|_| invalid("MPEG Surround fragment allocation failed"))?;
        for _ in 0..remaining / 8 {
            payload.push(bounded(&mut reader, 8, limit)? as u8);
        }
        // All fallible reads/checks finish before changing pending contents.
        // Reserve before append, so a failed continuation retains the message
        // without cloning its entire accumulated prefix on every fragment.
        let result = match segment {
            2 => {
                if self.pending.is_some() {
                    return Err(invalid("MPEG Surround start while a fragment is pending"));
                }
                self.pending = Some(payload);
                None
            }
            0 | 1 => {
                let pending = self
                    .pending
                    .as_mut()
                    .ok_or_else(|| invalid("MPEG Surround continuation/stop without start"))?;
                pending
                    .try_reserve(payload.len())
                    .map_err(|_| invalid("MPEG Surround reassembly allocation failed"))?;
                pending.extend(payload);
                if segment == 1 {
                    Some(Message {
                        extension,
                        kind,
                        payload: self.pending.take().unwrap(),
                    })
                } else {
                    None
                }
            }
            3 => {
                if let Some(prefix) = &self.pending {
                    let replacements = self
                        .replacements
                        .checked_add(1)
                        .ok_or_else(|| invalid("MPEG Surround replacement count overflow"))?;
                    let losses = if payload.starts_with(prefix) {
                        self.prefix_losses
                    } else {
                        self.prefix_losses
                            .checked_add(1)
                            .ok_or_else(|| invalid("MPEG Surround prefix-loss count overflow"))?
                    };
                    self.replacements = replacements;
                    self.prefix_losses = losses;
                }
                self.pending = None;
                Some(Message {
                    extension,
                    kind,
                    payload,
                })
            }
            _ => unreachable!(),
        };
        *bits = reader;
        Ok(result)
    }
}

impl Assembler {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        if let Some(pending) = &self.pending {
            f.vector(pending)?;
        }
        Ok(())
    }
}

impl Assembler {
    /// Owned pending-fragment payload, including spare capacity. Does not count
    /// caller-owned packets, stack objects or allocation headers.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        self.retained_payload_bytes_with_checkpoint(None)
    }
    pub fn retained_payload_bytes_with_checkpoint(&self, saved: Option<&Self>) -> Result<usize> {
        let mut footprint = super::memory::Footprint::new();
        self.visit_retained(&mut footprint)
            .map_err(|e| invalid(&e))?;
        if let Some(saved) = saved.filter(|saved| !std::ptr::eq(*saved, self)) {
            saved
                .visit_retained(&mut footprint)
                .map_err(|e| invalid(&e))?;
        }
        Ok(footprint.total())
    }
}

impl Assembler {
    /// Buffer/absolute-bit facade for AAC owners with a distinct cursor type.
    /// No source-buffer copy or implicit alignment; returned position follows the
    /// parsed envelope. Errors preserve pending ancillary state.
    pub fn read_bytes(
        &mut self,
        bytes: &[u8],
        start: usize,
        end: usize,
    ) -> Result<(Option<Message>, usize)> {
        let mut bits = BitReader::with_bit_len(bytes, end)?;
        bits.skip(start)?;
        let message = self.read(&mut bits, end)?;
        Ok((message, bits.position()))
    }
}
