struct Instance {
    bytes: Vec<u8>,
    bits: usize,
    position: usize,
}
struct Span {
    instance: usize,
    start: usize,
    end: usize,
}
pub(crate) struct Reader {
    instances: Vec<Instance>,
    next: usize,
    current: [Option<usize>; 5],
    spans: Vec<Span>,
}
impl Reader {
    fn new(frame: &RecoveredFrame) -> Result<Self> {
        let mut instances = Vec::new();
        for class in &frame.classes {
            if class.bits.iter().any(|&b| b > 1) {
                return Err(invalid("invalid ESC information bit"));
            }
            let mut bytes = vec![0; class.bits.len().div_ceil(8)];
            for (at, &bit) in class.bits.iter().enumerate() {
                bytes[at / 8] |= bit << (7 - at % 8);
            }
            instances.push(Instance {
                bytes,
                bits: class.bits.len(),
                position: 0,
            });
        }
        Ok(Self {
            instances,
            next: 0,
            current: [None; 5],
            spans: Vec::new(),
        })
    }
    fn allocate(&mut self, category: usize) -> Result<()> {
        if self.next == self.instances.len() {
            return Err(invalid("missing AAC ESC instance"));
        }
        self.current[category] = Some(self.next);
        self.next += 1;
        Ok(())
    }
    pub(crate) fn common(&mut self) -> Result<()> {
        self.allocate(0)
    }
    pub(crate) fn channel(&mut self, rvlc: bool) -> Result<()> {
        self.current[1..].fill(None);
        self.allocate(1)?;
        if rvlc {
            self.allocate(2)?;
        }
        Ok(())
    }
    pub(crate) fn payload(&mut self, tns: bool) -> Result<()> {
        if tns {
            self.allocate(3)?;
        }
        self.allocate(4)
    }
    pub(crate) fn read<T>(
        &mut self,
        category: usize,
        f: impl FnOnce(&mut BitReader<'_>) -> Result<T>,
    ) -> Result<T> {
        let index = self.current[category].ok_or_else(|| invalid("unassigned AAC ESC category"))?;
        let source = &mut self.instances[index];
        let mut bits = BitReader::with_bit_len(&source.bytes, source.bits)?;
        bits.skip(source.position)?;
        let output = f(&mut bits)?;
        let end = bits.position();
        self.spans.push(Span {
            instance: index,
            start: source.position,
            end,
        });
        source.position = end;
        Ok(output)
    }
    fn finish(self) -> Result<Vec<u8>> {
        if self.next != self.instances.len() {
            return Err(invalid("unconsumed AAC ESC instances"));
        }
        let count = self
            .spans
            .iter()
            .try_fold(0usize, |sum, s| sum.checked_add(s.end - s.start))
            .ok_or_else(|| invalid("AAC ESC payload length overflow"))?;
        for (index, instance) in self.instances.iter().enumerate() {
            let remaining = instance.bits - instance.position;
            // Only the last instance may carry final byte_alignment bits.
            if remaining > 0 {
                if index + 1 != self.instances.len() || remaining != (8 - count % 8) % 8 {
                    return Err(invalid("unconsumed AAC ESC information bits"));
                }
                for at in instance.position..instance.bits {
                    if instance.bytes[at / 8] >> (7 - at % 8) & 1 != 0 {
                        return Err(invalid("nonzero AAC ESC alignment bits"));
                    }
                }
            }
        }
        let mut raw = vec![0; count.div_ceil(8)];
        let mut out = 0;
        for span in self.spans {
            let bytes = &self.instances[span.instance].bytes;
            for at in span.start..span.end {
                raw[out / 8] |= ((bytes[at / 8] >> (7 - at % 8)) & 1) << (7 - out % 8);
                out += 1;
            }
        }
        Ok(raw)
    }
}
pub(crate) enum ChannelInput<'a, 'b> {
    Logical(&'b mut BitReader<'a>),
    Categories(&'b mut Reader),
}
impl ChannelInput<'_, '_> {
    pub(crate) fn read<T>(
        &mut self,
        category: usize,
        f: impl FnOnce(&mut BitReader<'_>) -> Result<T>,
    ) -> Result<T> {
        match self {
            Self::Logical(bits) => f(bits),
            Self::Categories(reader) => reader.read(category, f),
        }
    }
    pub(crate) fn payload(&mut self, tns: bool) -> Result<()> {
        match self {
            Self::Logical(_) => Ok(()),
            Self::Categories(reader) => reader.payload(tns),
        }
    }
}
/// Restore a core-only ELD logical AU from ordered ESC instances. This parses
/// syntax using the shared channel readers; PCM/history are decoded only once
/// by the normal transactional ELD decoder. LD-SBR ESC 9/10 is separate work.
pub fn reassemble_eld_core(config: &AacConfig, frame: &RecoveredFrame) -> Result<Vec<u8>> {
    if config.object_type != 39 {
        return Err(invalid("ELD ESC mapping requires AOT39"));
    }
    let kinds: &[u8] = match config.channel_configuration {
        1 => &[0],
        2 => &[1],
        3 => &[0, 1],
        4 => &[0, 1, 0],
        5 => &[0, 1, 1],
        6 => &[0, 1, 1, 3],
        7 | 12 => &[0, 1, 1, 1, 3],
        11 => &[0, 1, 1, 0, 3],
        14 => &[0, 1, 1, 3, 1],
        _ => return Err(invalid("unsupported ELD ESC layout")),
    };
    let mut reader = Reader::new(frame)?;
    for &kind in kinds {
        if kind == 1 {
            super::aac_pair::ChannelPair::read_eld_esc(&mut reader, config)?;
        } else {
            super::aac_channel::ChannelData::read_eld_esc(&mut reader, config, None, false)?;
        }
    }
    reader.finish()
}
