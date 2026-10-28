struct ErSbrContext<'a> {
    kinds: &'a [u8],
    headers: &'a [Option<SbrHeader>],
    rate: u32,
    slots: u8,
}
impl ErSbrContext<'_> {
    fn group(&self, ordinal: usize) -> Result<(usize, Option<SbrHeader>)> {
        let mut offset = 0;
        let mut group = 0;
        for &kind in self.kinds {
            let width = if kind == 1 { 2 } else { 1 };
            if kind != 3 {
                if group == ordinal {
                    return Ok((width, self.headers.get(offset).cloned().flatten()));
                }
                group += 1;
            }
            offset += width;
        }
        Err(invalid("excess ER AAC SBR element"))
    }
}
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
    current: [Option<usize>; 11],
    spans: Vec<Span>,
    grouped_ep1: bool,
    // EP1 category boundaries are bound only when their syntax is first read.
    pending: [bool; 11],
    last: Option<usize>,
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
            current: [None; 11],
            spans: Vec::new(),
            grouped_ep1: false,
            pending: [false; 11],
            last: None,
        })
    }
    fn allocate(&mut self, category: usize) -> Result<()> {
        if self.grouped_ep1 {
            self.current[category] = None;
            self.pending[category] = true;
            return Ok(());
        }
        if self.next == self.instances.len() {
            return Err(invalid("missing AAC ESC instance"));
        }
        self.current[category] = Some(self.next);
        self.next += 1;
        Ok(())
    }
    fn bind(&mut self, category: usize, empty_allowed: bool) -> Result<usize> {
        if let Some(index) = self.current[category] {
            return Ok(index);
        }
        if !self.pending[category] {
            return Err(invalid("unassigned AAC ESC category"));
        }
        let continued = self
            .last
            .filter(|&i| self.instances[i].position < self.instances[i].bits);
        let index = if let Some(index) = continued {
            index
        } else if self.next < self.instances.len() {
            let index = self.next;
            self.next += 1;
            index
        } else if empty_allowed {
            self.last
                .ok_or_else(|| invalid("missing AAC ESC instance"))?
        } else {
            return Err(invalid("missing AAC ESC instance"));
        };
        self.pending[category] = false;
        self.current[category] = Some(index);
        self.last = Some(index);
        Ok(index)
    }
    fn sbr(&mut self) -> Result<()> {
        self.allocate(9)?;
        self.allocate(10)
    }
    fn read_sbr<T>(
        &mut self,
        category: usize,
        f: impl FnOnce(&mut SbrBitReader<'_>) -> SbrResult<T>,
    ) -> Result<T> {
        self.bind(category, false)?;
        let index =
            self.current[category].ok_or_else(|| invalid("unassigned LD-SBR ESC category"))?;
        let source = &mut self.instances[index];
        let mut bits = SbrBitReader::with_bit_len(&source.bytes, source.bits)
            .map_err(|e| invalid(&e.to_string()))?;
        bits.skip(source.position)
            .map_err(|e| invalid(&e.to_string()))?;
        let output = f(&mut bits).map_err(|e| invalid(&e.to_string()))?;
        let end = bits.position();
        self.spans.push(Span {
            instance: index,
            start: source.position,
            end,
        });
        source.position = end;
        Ok(output)
    }
    pub(crate) fn common(&mut self) -> Result<()> {
        self.allocate(0)
    }
    pub(crate) fn channel(&mut self, rvlc: bool) -> Result<()> {
        self.current[1..].fill(None);
        self.pending[1..].fill(false);
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
        let previous = self.last;
        let was_pending = self.pending[category];
        let missing = was_pending
            && self.next == self.instances.len()
            && previous.is_some_and(|i| self.instances[i].position == self.instances[i].bits);
        let index = self.bind(category, true)?;
        let final_instance = index + 1 == self.instances.len();
        let source = &mut self.instances[index];
        let mut bits = BitReader::with_bit_len(&source.bytes, source.bits)?;
        bits.skip(source.position)?;
        let output = f(&mut bits).map_err(|e| {
            if missing {
                invalid("missing AAC ESC instance")
            } else {
                e
            }
        })?;
        let end = bits.position();
        // A grammar-proven empty category may be grouped at an AU boundary.
        // Do not claim the next nonempty AU when no bit was read. Explicit
        // empty AUs still retain their own transport boundary.
        if self.grouped_ep1
            && was_pending
            && end == source.position
            && source.bits != 0
            && previous != Some(index)
        {
            let count = self
                .spans
                .iter()
                .try_fold(0usize, |sum, span| sum.checked_add(span.end - span.start))
                .ok_or_else(|| invalid("AAC ESC payload length overflow"))?;
            // A final empty instance can still carry the grammar-required AAC
            // alignment. Keep that declared terminal AU, including corrupt
            // alignment values which finish() must reject rather than hide.
            if !final_instance || source.bits != (8 - count % 8) % 8 {
                self.next -= 1;
                self.last = previous;
                self.current[category] = previous;
            }
        }
        self.spans.push(Span {
            instance: index,
            start: source.position,
            end,
        });
        source.position = end;
        Ok(output)
    }
    fn extension_field(
        &mut self,
        category: usize,
        width: u8,
        remaining: &mut usize,
    ) -> Result<u32> {
        if usize::from(width) > *remaining {
            return Err(invalid("AAC extension exceeds fill payload"));
        }
        let value = self.read(category, |bits| bits.read(width))?;
        *remaining -= usize::from(width);
        Ok(value)
    }
    fn extension_skip(
        &mut self,
        category: usize,
        count: usize,
        remaining: &mut usize,
    ) -> Result<()> {
        if count > *remaining {
            return Err(invalid("AAC extension exceeds fill payload"));
        }
        self.read(category, |bits| {
            if count > bits.remaining() {
                return Err(invalid("AAC ESC extension exceeds category boundary"));
            }
            bits.skip(count)
        })?;
        *remaining -= count;
        Ok(())
    }
    fn opaque_extension(&mut self, remaining: &mut usize) -> Result<()> {
        if *remaining != 0 {
            self.allocate(7)?;
            self.extension_skip(7, *remaining, remaining)?;
        }
        Ok(())
    }
    /// ER extension_payload records occupy whole bytes after the main payload;
    /// final AAC byte_alignment remains outside that area and in the last ESC.
    fn extensions(&mut self, sbr: Option<&ErSbrContext<'_>>, grouped: bool) -> Result<()> {
        if !grouped && self.next == self.instances.len() {
            return Ok(());
        }
        if !grouped
            && self.instances[..self.next]
                .iter()
                .any(|i| i.bits != i.position)
        {
            return Err(invalid("unconsumed AAC ESC information bits"));
        }
        let core = self
            .spans
            .iter()
            .try_fold(0usize, |sum, span| sum.checked_add(span.end - span.start))
            .ok_or_else(|| invalid("AAC ESC payload length overflow"))?;
        let rest = if grouped {
            &self.instances[..]
        } else {
            &self.instances[self.next..]
        }
        .iter()
        .try_fold(0usize, |sum, i| sum.checked_add(i.bits - i.position))
        .ok_or_else(|| invalid("AAC ESC payload length overflow"))?;
        if grouped && rest < 8 {
            return Ok(());
        }
        if core.checked_add(rest).is_none_or(|n| n % 8 != 0) {
            return Err(invalid("AAC ESC extended payload is not byte aligned"));
        }
        let mut remaining = rest / 8 * 8;
        let mut sbr_index = 0;
        while remaining != 0 {
            self.current[5..9].fill(None);
            self.allocate(5)?;
            let kind = self.extension_field(5, 4, &mut remaining)?;
            if kind == 14 {
                return Err(invalid("ER AAC SBR CRC is forbidden"));
            }
            if sbr_index != 0 && kind != 13 {
                return Err(invalid("ER AAC extensions must precede SBR"));
            }
            match kind {
                0 => self.opaque_extension(&mut remaining)?,
                1 => {
                    self.allocate(7)?;
                    if self.extension_field(7, 4, &mut remaining)? != 0 {
                        return Err(invalid("invalid AAC fill-data nibble"));
                    }
                    while remaining != 0 {
                        if self.extension_field(7, 8, &mut remaining)? != 0xa5 {
                            return Err(invalid("invalid AAC fill-data byte"));
                        }
                    }
                }
                2 => {
                    if self.extension_field(5, 4, &mut remaining)? != 0 {
                        self.opaque_extension(&mut remaining)?;
                        continue;
                    }
                    self.allocate(8)?;
                    let mut length = 0usize;
                    loop {
                        let part = self.extension_field(8, 8, &mut remaining)? as usize;
                        length = length
                            .checked_add(part)
                            .ok_or_else(|| invalid("AAC ancillary data length overflow"))?;
                        if part != 255 {
                            break;
                        }
                    }
                    if length > remaining / 8 {
                        return Err(invalid("AAC ancillary data exceeds fill payload"));
                    }
                    self.extension_skip(8, length * 8, &mut remaining)?;
                }
                11 => {
                    self.allocate(6)?;
                    super::aac_pce::read_dynamic_range(|width| {
                        self.extension_field(6, width, &mut remaining)
                    })?;
                }
                12 => {
                    return Err(unsupported("AAC fill extension tool is not implemented"));
                }
                13 => {
                    let context = sbr.ok_or_else(|| {
                        unsupported("AAC ESC SBR category mapping is not yet implemented")
                    })?;
                    let (channels, header) = context.group(sbr_index)?;
                    self.allocate(9)?;
                    let index = self.bind(9, false)?;
                    let source = &self.instances[index];
                    let mut available = remaining.min(source.bits - source.position);
                    if grouped {
                        // An ER SBR payload occupies whole bytes including its
                        // preceding four-bit extension ID. A grouped AU can end
                        // after a later extension's ID; bound this semantic view
                        // at the last complete payload-sized region in the AU.
                        if available < 4 {
                            return Err(invalid("truncated ER SBR ESC payload"));
                        }
                        available -= (available + 4) % 8;
                    }
                    let (used, exact) = self.read_sbr(9, |bits| {
                        let start = bits.position();
                        let end = start + available;
                        let mut syntax = SbrSyntax::with_header(header);
                        syntax.read(bits, end, false, context.rate, context.slots, channels)?;
                        Ok((bits.position() - start, bits.position() == end))
                    })?;
                    if !exact && !grouped {
                        return Err(invalid("unconsumed ER SBR ESC information bits"));
                    }
                    remaining -= used;
                    sbr_index += 1;
                }
                14 => return Err(invalid("ER AAC SBR CRC is forbidden")),
                _ => self.opaque_extension(&mut remaining)?,
            }
        }
        if sbr_index != 0
            && sbr.is_some_and(|c| c.kinds.iter().filter(|&&k| k != 3).count() != sbr_index)
        {
            return Err(invalid("missing ER AAC SBR element"));
        }
        Ok(())
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
            if index + 1 == self.instances.len() && remaining < (8 - count % 8) % 8 {
                return Err(invalid("incomplete AAC ESC alignment bits"));
            }
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
    pub(crate) fn grouped_tns_mark(&self) -> Option<usize> {
        match self {
            Self::Categories(reader) if reader.grouped_ep1 => Some(reader.spans.len()),
            _ => None,
        }
    }
    pub(crate) fn restore_tns_order(&mut self, mark: usize) {
        if let Self::Categories(reader) = self {
            // Category headers precede TNS in transport order, while the logical
            // ELD packet puts TNS before those remaining header/RVLC fields.
            reader.spans[mark..].rotate_right(1);
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
/// by the normal transactional ELD decoder.
pub fn reassemble_eld_core(config: &AacConfig, frame: &RecoveredFrame) -> Result<Vec<u8>> {
    reassemble_eld(config, frame, None)
}
/// Restore the ER-LC main payload in its original logical order. Extension
/// instances must be consumed by their own semantic mapping, never appended
/// blindly as if their category order were the logical bitstream order.
pub fn reassemble_er_lc(
    config: &AacConfig,
    program: Option<&super::aac_pce::ProgramConfig>,
    frame: &RecoveredFrame,
) -> Result<Vec<u8>> {
    if config.object_type != 17 {
        return Err(invalid("ER-LC ESC mapping requires AOT17"));
    }
    reassemble_er(config, program, frame)
}
/// Restore ER LC/LTP/LD main syntax, including deferred channel predictors.
/// Parsing records original spans; only normal decoding updates LTP history.
/// No prior SBR headers are assumed; use `reassemble_er_with_sbr` for continuation.
pub fn reassemble_er(
    config: &AacConfig,
    program: Option<&super::aac_pce::ProgramConfig>,
    frame: &RecoveredFrame,
) -> Result<Vec<u8>> {
    reassemble_er_with_sbr(config, program, frame, &[])
}

/// Reconstruct with immutable current headers indexed by coded channel offset.
/// A CPE uses its first channel slot; LFE advances the offset but has no SBR.
/// This parser does not commit synthesis, coefficient or header history.
pub fn reassemble_er_with_sbr(
    config: &AacConfig,
    program: Option<&super::aac_pce::ProgramConfig>,
    frame: &RecoveredFrame,
    headers: &[Option<SbrHeader>],
) -> Result<Vec<u8>> {
    reassemble_er_transport(config, program, frame, headers, false)
}
/// EP1 transports consecutive ER core and extension instances in grouped AUs.
/// EP3 retains the separate-class entrypoint above.
pub fn reassemble_er_ep1(
    config: &AacConfig,
    program: Option<&super::aac_pce::ProgramConfig>,
    frame: &RecoveredFrame,
    headers: &[Option<SbrHeader>],
) -> Result<Vec<u8>> {
    reassemble_er_transport(config, program, frame, headers, true)
}
fn reassemble_er_transport(
    config: &AacConfig,
    program: Option<&super::aac_pce::ProgramConfig>,
    frame: &RecoveredFrame,
    headers: &[Option<SbrHeader>],
    grouped: bool,
) -> Result<Vec<u8>> {
    if !matches!(config.object_type, 17 | 19 | 23) {
        return Err(invalid("ER ESC mapping requires AOT17/19/23"));
    }
    let configured: Vec<u8>;
    let kinds: &[u8] = match config.channel_configuration {
        0 => {
            let program = program.ok_or_else(|| invalid("missing ER ESC PCE layout"))?;
            configured = program
                .elements
                .iter()
                .map(|e| {
                    if e.position == super::aac_pce::Position::Lfe {
                        3
                    } else {
                        u8::from(e.pair)
                    }
                })
                .collect();
            &configured
        }
        1 => &[0],
        2 => &[1],
        3 => &[0, 1],
        4 => &[0, 1, 0],
        5 => &[0, 1, 1],
        6 => &[0, 1, 1, 3],
        7 | 12 => &[0, 1, 1, 1, 3],
        11 => &[0, 1, 1, 0, 3],
        14 => &[0, 1, 1, 3, 1],
        _ => return Err(invalid("unsupported ER ESC layout")),
    };
    let mut reader = Reader::new(frame)?;
    reader.grouped_ep1 = grouped;
    for &kind in kinds {
        if kind == 1 {
            super::aac_pair::ChannelPair::read_er_esc(&mut reader, config)?;
        } else {
            super::aac_channel::ChannelData::read_er_esc(&mut reader, config, None, false, false)?;
        }
    }
    let context = ErSbrContext {
        kinds,
        headers,
        rate: config
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("SBR frequency overflow"))?,
        slots: (config.frame_samples / 64) as u8,
    };
    reader.extensions(
        matches!(config.object_type, 17 | 19).then_some(&context),
        grouped,
    )?;
    reader.finish()
}
/// Restore ELD with optional current LD-SBR context. Header history is read
/// immutably here; the normal decoder alone commits the reconstructed packet.
pub fn reassemble_eld(
    config: &AacConfig,
    frame: &RecoveredFrame,
    sbr: Option<&SbrDecoder>,
) -> Result<Vec<u8>> {
    reassemble_eld_transport(config, frame, sbr, false)
}
/// EP1 AUs can contain successive ELD core and LD-SBR instances across elements.
/// EP3 recovered classes retain their declared separate-instance boundaries
/// through the ordinary entrypoint above.
pub fn reassemble_eld_ep1(
    config: &AacConfig,
    frame: &RecoveredFrame,
    sbr: Option<&SbrDecoder>,
) -> Result<Vec<u8>> {
    reassemble_eld_transport(config, frame, sbr, true)
}
fn reassemble_eld_transport(
    config: &AacConfig,
    frame: &RecoveredFrame,
    sbr: Option<&SbrDecoder>,
    grouped_ep1: bool,
) -> Result<Vec<u8>> {
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
    reader.grouped_ep1 = grouped_ep1;
    for &kind in kinds {
        if kind == 1 {
            super::aac_pair::ChannelPair::read_eld_esc(&mut reader, config)?;
        } else {
            super::aac_channel::ChannelData::read_eld_esc(&mut reader, config, None, false)?;
        }
    }
    if let Some(sbr) = sbr {
        let headers = sbr.esc_headers(
            config.sample_rate,
            config.frame_samples,
            config.channel_configuration,
        )?;
        let rate = config
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("LD-SBR ESC frequency overflow"))?;
        let slots = (config.frame_samples / 32) as u8;
        let mut headers = headers.into_iter();
        for &kind in kinds.iter().filter(|&&kind| kind != 3) {
            let header = headers
                .next()
                .ok_or_else(|| invalid("missing LD-SBR ESC header"))?;
            reader.sbr()?;
            let prefix = reader.read_sbr(9, |bits| {
                if sbr.initial_config().crc {
                    bits.read(10)?;
                }
                let end = bits.position() + bits.remaining();
                let header = if bits.bit()? {
                    SbrHeader::read(bits, end)?
                } else {
                    header
                };
                SbrData::read_low_delay_prefix(
                    bits,
                    end,
                    &header,
                    rate,
                    slots,
                    if kind == 1 { 2 } else { 1 },
                )
            })?;
            reader.read_sbr(10, |bits| {
                let end = bits.position() + bits.remaining();
                prefix.finish(bits, end)
            })?;
        }
    }
    reader.extensions(None, false)?;
    reader.finish()
}
