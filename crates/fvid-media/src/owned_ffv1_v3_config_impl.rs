/// One independently initialized FFV1 v3 quantizer/context set.
pub struct QuantizationSet {
    tables: [[i32; 256]; 5],
    initial: Vec<[u8; 32]>,
}
impl QuantizationSet {
    pub fn context_count(&self) -> usize {
        self.initial.len()
    }
    pub fn quantizers(&self) -> &[[i32; 256]; 5] {
        &self.tables
    }
    pub fn initial_states(&self) -> &[[u8; 32]] {
        &self.initial
    }
}
/// Validated configuration record, independent of per-frame adaptation history.
pub struct Configuration {
    micro_version: u32,
    coder: u8,
    rgb: bool,
    depth: u8,
    chroma: bool,
    alpha: bool,
    shifts: [u8; 2],
    grid: [u32; 2],
    sets: Vec<QuantizationSet>,
    transitions: [u8; 256],
    ec: bool,
    intra: bool,
    memory: usize,
}
fn ffv1_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = (crc << 1)
                ^ if crc & 0x8000_0000 != 0 {
                    0x04c1_1db7
                } else {
                    0
                };
        }
    }
    crc
}
impl Configuration {
    /// CRC is verified before any range-coded parameter is read or allocated.
    pub fn parse(data: &[u8], budget: usize) -> Result<Self> {
        if data.len() < 6 {
            return Err(invalid("truncated FFV1 configuration record"));
        }
        if ffv1_crc(data) != 0 {
            return Err(invalid("FFV1 configuration CRC mismatch"));
        }
        let mut r = Range::new(&data[..data.len() - 4])?;
        let mut h = [128; 32];
        if r.integer(&mut h, false)? != 3 {
            return Err(unsupported("FFV1 configuration requires version 3"));
        }
        let micro_version = r.integer(&mut h, false)? as u32;
        if micro_version < 4 {
            return Err(unsupported("experimental FFV1 v3 micro-version"));
        }
        let coder = r.integer(&mut h, false)?;
        if !(0..=2).contains(&coder) {
            return Err(unsupported("unsupported FFV1 entropy coder"));
        }
        let mut transitions = TRANSITION;
        if coder == 2 {
            for i in 1..256 {
                let value = i64::from(TRANSITION[i]) + i64::from(r.integer(&mut h, true)?);
                if !(0..=255).contains(&value) {
                    return Err(invalid("invalid FFV1 custom transition"));
                }
                transitions[i] = value as u8;
            }
        }
        let rgb = match r.integer(&mut h, false)? {
            0 => false,
            1 => true,
            _ => return Err(unsupported("unsupported FFV1 color space")),
        };
        let depth = match r.integer(&mut h, false)? {
            0 => 8,
            n @ 8..=16 => n as u8,
            _ => return Err(unsupported("unsupported FFV1 sample depth")),
        };
        let chroma = r.bit(&mut h[0])?;
        let sx = r.integer(&mut h, false)?;
        let sy = r.integer(&mut h, false)?;
        if sx > 4 || sy > 4 {
            return Err(invalid("invalid FFV1 chroma shift"));
        }
        let alpha = r.bit(&mut h[0])?;
        if rgb && (!chroma || sx != 0 || sy != 0) {
            return Err(invalid("invalid FFV1 RGB chroma configuration"));
        }
        let grid = [
            r.integer(&mut h, false)? as u32 + 1,
            r.integer(&mut h, false)? as u32 + 1,
        ];
        let count = r.integer(&mut h, false)? as usize;
        if !(1..=8).contains(&count) {
            return Err(invalid("invalid FFV1 quantizer set count"));
        }
        let mut memory = std::mem::size_of::<Self>()
            .checked_add(count * std::mem::size_of::<QuantizationSet>())
            .ok_or_else(|| invalid("FFV1 configuration storage overflow"))?;
        if memory > budget {
            return Err(invalid("FFV1 configuration exceeds memory budget"));
        }
        let mut sets = Vec::new();
        sets.try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate FFV1 quantizer sets"))?;
        for _ in 0..count {
            let mut tables = [[0; 256]; 5];
            let mut scale = 1i32;
            for table in &mut tables {
                let mut model = [128; 32];
                let mut at = 0;
                let mut value = 0;
                while at < 128 {
                    let run = r.integer(&mut model, false)? as usize + 1;
                    if run > 128 - at {
                        return Err(invalid("invalid FFV1 quantizer run"));
                    }
                    table[at..at + run].fill(scale * value);
                    at += run;
                    value += 1;
                }
                for i in 1..128 {
                    table[256 - i] = -table[i];
                }
                table[128] = -table[127];
                scale = scale
                    .checked_mul(2 * value - 1)
                    .filter(|&n| n <= 65535)
                    .ok_or_else(|| invalid("FFV1 context count overflow"))?;
            }
            let contexts = (scale as usize).div_ceil(2);
            memory = memory
                .checked_add(contexts * 32)
                .ok_or_else(|| invalid("FFV1 configuration storage overflow"))?;
            if memory > budget {
                return Err(invalid("FFV1 configuration exceeds memory budget"));
            }
            let mut initial = Vec::new();
            initial
                .try_reserve_exact(contexts)
                .map_err(|_| invalid("cannot allocate FFV1 initial states"))?;
            initial.resize(contexts, [128; 32]);
            sets.push(QuantizationSet { tables, initial });
        }
        // Each probability position has an independent scalar coding context.
        let mut states = [[128; 32]; 32];
        for set in &mut sets {
            if r.bit(&mut h[0])? {
                for j in 0..set.initial.len() {
                    for k in 0..32 {
                        let predictor = if j == 0 { 128 } else { set.initial[j - 1][k] };
                        let delta = r.integer(&mut states[k], true)?;
                        set.initial[j][k] = ((i64::from(predictor) + i64::from(delta)) & 255) as u8;
                    }
                }
            }
        }
        let ec = match r.integer(&mut h, false)? {
            0 => false,
            1 => true,
            _ => return Err(unsupported("unsupported FFV1 error correction mode")),
        };
        let intra = match r.integer(&mut h, false)? {
            0 => false,
            1 => true,
            _ => return Err(unsupported("unsupported FFV1 intra constraint")),
        };
        Ok(Self {
            micro_version,
            coder: coder as u8,
            rgb,
            depth,
            chroma,
            alpha,
            shifts: [sx as u8, sy as u8],
            grid,
            sets,
            transitions,
            ec,
            intra,
            memory,
        })
    }
    pub fn micro_version(&self) -> u32 {
        self.micro_version
    }
    pub fn coder_type(&self) -> u8 {
        self.coder
    }
    pub fn rgb(&self) -> bool {
        self.rgb
    }
    pub fn depth(&self) -> u8 {
        self.depth
    }
    pub fn chroma(&self) -> bool {
        self.chroma
    }
    pub fn alpha(&self) -> bool {
        self.alpha
    }
    pub fn chroma_shifts(&self) -> [u8; 2] {
        self.shifts
    }
    pub fn slice_grid(&self) -> [u32; 2] {
        self.grid
    }
    pub fn quantization_sets(&self) -> &[QuantizationSet] {
        &self.sets
    }
    pub fn transitions(&self) -> &[u8; 256] {
        &self.transitions
    }
    pub fn slice_crc(&self) -> bool {
        self.ec
    }
    pub fn intra(&self) -> bool {
        self.intra
    }
    pub fn memory_bytes(&self) -> usize {
        self.memory
    }
    /// Index slices backwards from their footers, verifying CRC and error status.
    /// No sample data is read or published by this framing operation.
    pub fn slices<'a>(&self, packet: &'a [u8], budget: usize) -> Result<Vec<Slice<'a>>> {
        let footer = if self.ec { 8 } else { 3 };
        let max = u64::from(self.grid[0]) * u64::from(self.grid[1]);
        let start_of = |end: usize| -> Result<usize> {
            if end < footer {
                return Err(invalid("truncated FFV1 slice footer"));
            }
            let at = end - footer;
            let length = usize::from(packet[at]) << 16
                | usize::from(packet[at + 1]) << 8
                | usize::from(packet[at + 2]);
            if length < 2 || length > at {
                return Err(invalid("invalid FFV1 slice size"));
            }
            Ok(at - length)
        };
        let mut count = 0usize;
        let mut end = packet.len();
        while end != 0 {
            let start = start_of(end)?;
            let size_at = end - footer;
            if self.ec {
                if ffv1_crc(&packet[start..end]) != 0 {
                    return Err(invalid("FFV1 slice CRC mismatch"));
                }
                if packet[size_at + 3] != 0 {
                    return Err(invalid("FFV1 slice reports damaged content"));
                }
            }
            if count as u64 >= max {
                return Err(invalid("FFV1 slice count exceeds raster"));
            }
            count += 1;
            let bytes = count
                .checked_mul(std::mem::size_of::<Slice<'_>>())
                .and_then(|n| n.checked_add(self.memory))
                .ok_or_else(|| invalid("FFV1 slice index storage overflow"))?;
            if bytes > budget {
                return Err(invalid("FFV1 slice index exceeds memory budget"));
            }
            end = start;
        }
        if count == 0 {
            return Err(invalid("FFV1 frame contains no slices"));
        }
        let mut slices = Vec::new();
        slices
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate FFV1 slice index"))?;
        end = packet.len();
        while end != 0 {
            let start = start_of(end)?;
            slices.push(Slice {
                payload: &packet[start..end - footer],
                offset: start,
            });
            end = start;
        }
        slices.reverse();
        Ok(slices)
    }
}
pub struct Slice<'a> {
    pub payload: &'a [u8],
    pub offset: usize,
}
