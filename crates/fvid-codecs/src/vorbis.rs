//! FVid-owned Vorbis I bitstream primitives.
//! This is not yet a complete audio decoder; setup, floor/residue and synthesis
//! must be implemented before replacing the existing playback decoder.
use crate::{Result, invalid};

/// LSB-first packet reader. Failed reads keep the cursor and latch end-of-packet.
pub struct PacketBits<'a> {
    bytes: &'a [u8],
    bit: usize,
    ended: bool,
}
impl<'a> PacketBits<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            bit: 0,
            ended: false,
        }
    }
    pub fn position(&self) -> usize {
        self.bit
    }
    pub fn skip(&mut self, width: usize) -> Result<()> {
        if self.ended {
            return Err(invalid("end of Vorbis packet"));
        }
        let end = self
            .bit
            .checked_add(width)
            .ok_or_else(|| invalid("Vorbis bit offset overflow"))?;
        if end / 8 > self.bytes.len() || (end / 8 == self.bytes.len() && end % 8 != 0) {
            self.ended = true;
            return Err(invalid("truncated Vorbis packet"));
        }
        self.bit = end;
        Ok(())
    }
    pub fn read(&mut self, width: u8) -> Result<u32> {
        if self.ended {
            return Err(invalid("end of Vorbis packet"));
        }
        if width > 32 {
            return Err(invalid("Vorbis bit read exceeds 32 bits"));
        }
        let end = self
            .bit
            .checked_add(usize::from(width))
            .ok_or_else(|| invalid("Vorbis bit offset overflow"))?;
        if end / 8 > self.bytes.len() || (end / 8 == self.bytes.len() && end % 8 != 0) {
            self.ended = true;
            return Err(invalid("truncated Vorbis packet"));
        }
        let mut value = 0;
        for shift in 0..width {
            let at = self.bit + usize::from(shift);
            value |= u32::from((self.bytes[at / 8] >> (at % 8)) & 1) << shift;
        }
        self.bit = end;
        Ok(value)
    }
}

/// Identification fields; bitrate values are signed, advisory hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identification {
    pub channels: u8,
    pub sample_rate: u32,
    pub bitrate_maximum: i32,
    pub bitrate_nominal: i32,
    pub bitrate_minimum: i32,
    pub block_exponents: [u8; 2],
}
impl Identification {
    pub fn parse(packet: &[u8]) -> Result<Self> {
        let mut bits = PacketBits::new(packet);
        if bits.read(8)? != 1 {
            return Err(invalid("Vorbis identification packet type is not 1"));
        }
        for expected in b"vorbis" {
            if bits.read(8)? != u32::from(*expected) {
                return Err(invalid("invalid Vorbis identification signature"));
            }
        }
        let version = bits.read(32)?;
        if version != 0 {
            return Err(invalid(&format!("unsupported Vorbis version {version}")));
        }
        let channels = bits.read(8)? as u8;
        let sample_rate = bits.read(32)?;
        let bitrate_maximum = bits.read(32)? as i32;
        let bitrate_nominal = bits.read(32)? as i32;
        let bitrate_minimum = bits.read(32)? as i32;
        let block_exponents = [bits.read(4)? as u8, bits.read(4)? as u8];
        if channels == 0 || sample_rate == 0 {
            return Err(invalid("Vorbis channels and sample rate must be nonzero"));
        }
        if block_exponents.iter().any(|e| !(6..=13).contains(e))
            || block_exponents[0] > block_exponents[1]
        {
            return Err(invalid("invalid Vorbis block size exponents"));
        }
        if bits.read(1)? != 1 {
            return Err(invalid("missing Vorbis identification framing bit"));
        }
        Ok(Self {
            channels,
            sample_rate,
            bitrate_maximum,
            bitrate_nominal,
            bitrate_minimum,
            block_exponents,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(channels: u8, blocks: u8) -> Vec<u8> {
        let mut h = b"\x01vorbis".to_vec();
        h.extend_from_slice(&0u32.to_le_bytes());
        h.push(channels);
        h.extend_from_slice(&48000u32.to_le_bytes());
        for n in [-1i32, 96000, -1] {
            h.extend_from_slice(&n.to_le_bytes());
        }
        h.extend_from_slice(&[blocks, 1]);
        h
    }
    #[test]
    fn all_identification_channels_and_block_pairs_are_owned() {
        for channels in 1..=255 {
            for short in 6..=13 {
                for long in short..=13 {
                    let h = header(channels, short | long << 4);
                    let id = Identification::parse(&h).unwrap();
                    assert_eq!(id.channels, channels);
                    assert_eq!(id.block_exponents, [short, long]);
                    assert_eq!(id.bitrate_maximum, -1);
                    assert_eq!(id.bitrate_nominal, 96000);
                }
            }
        }
    }
    #[test]
    fn identification_checks_every_truncation_and_required_field() {
        let h = header(2, 0xb8);
        for end in 0..h.len() {
            assert!(Identification::parse(&h[..end]).is_err(), "end={end}");
        }
        for (at, value) in [(0, 3), (1, 0), (7, 1), (11, 0), (28, 0x68), (29, 0)] {
            let mut bad = h.clone();
            bad[at] = value;
            assert!(Identification::parse(&bad).is_err());
        }
        let mut bad = h.clone();
        bad[12..16].fill(0);
        assert!(Identification::parse(&bad).is_err());
        let mut padded = h;
        padded[29] = 255;
        padded.extend_from_slice(&[17, 99]);
        assert!(Identification::parse(&padded).is_ok());
    }
    #[test]
    fn bitpacking_matches_independent_integer_windows_and_is_transactional() {
        let bytes = [0x91, 0x37, 0x80, 0xfa, 0x15, 0x62, 0x44, 0xb9];
        let word = u64::from_le_bytes(bytes);
        for start in 0..=64 {
            for width in 0..=32u8 {
                let mut bits = PacketBits::new(&bytes);
                for _ in 0..start {
                    bits.read(1).unwrap();
                }
                let before = bits.position();
                let got = bits.read(width);
                if start + usize::from(width) <= 64 {
                    let expected = if width == 0 {
                        0
                    } else {
                        ((word >> start) & ((1u64 << width) - 1)) as u32
                    };
                    assert_eq!(got.unwrap(), expected);
                } else {
                    assert!(got.is_err());
                    assert_eq!(bits.position(), before);
                }
            }
        }
    }
}

/// Packed codeword-length section, before the vector lookup portion of a book.
/// Zero lengths identify unused sparse entries. Tree validation follows later.
#[derive(Debug, PartialEq, Eq)]
pub struct CodewordLengths {
    pub dimensions: u16,
    pub lengths: Vec<u8>,
}
impl CodewordLengths {
    /// The caller supplies the available entry allocation budget.
    pub fn parse(bits: &mut PacketBits<'_>, max_entries: usize) -> Result<Self> {
        if bits.read(24)? != 0x564342 {
            return Err(invalid("invalid Vorbis codebook sync"));
        }
        let dimensions = bits.read(16)? as u16;
        let entries = bits.read(24)? as usize;
        if dimensions == 0 || entries == 0 {
            return Err(invalid("empty Vorbis codebook geometry"));
        }
        if entries > max_entries {
            return Err(invalid("Vorbis codebook exceeds entry budget"));
        }
        let mut lengths = Vec::new();
        lengths
            .try_reserve_exact(entries)
            .map_err(|_| invalid("cannot allocate Vorbis codeword lengths"))?;
        if bits.read(1)? == 0 {
            let sparse = bits.read(1)? != 0;
            for _ in 0..entries {
                lengths.push(if sparse && bits.read(1)? == 0 {
                    0
                } else {
                    bits.read(5)? as u8 + 1
                });
            }
        } else {
            let mut length = bits.read(5)? as u8 + 1;
            while lengths.len() < entries {
                if length > 32 {
                    return Err(invalid("Vorbis codeword length exceeds 32"));
                }
                let remaining = entries - lengths.len();
                let width = (usize::BITS - remaining.leading_zeros()) as u8;
                let count = bits.read(width)? as usize;
                if count > remaining {
                    return Err(invalid("Vorbis ordered run exceeds entries"));
                }
                lengths.resize(lengths.len() + count, length);
                length += 1;
            }
        }
        Ok(Self {
            dimensions,
            lengths,
        })
    }
}

#[cfg(test)]
mod length_tests {
    use super::*;
    fn packed(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut bit = 0;
        for &(value, width) in fields {
            for shift in 0..width {
                if bit % 8 == 0 {
                    data.push(0);
                }
                data[bit / 8] |= (((value >> shift) & 1) as u8) << (bit % 8);
                bit += 1;
            }
        }
        data
    }
    #[test]
    fn end_of_packet_is_sticky_even_for_zero_bits() {
        let mut bits = PacketBits::new(&[42]);
        assert_eq!(bits.read(8).unwrap(), 42);
        assert_eq!(bits.read(0).unwrap(), 0);
        assert!(bits.read(1).is_err());
        assert_eq!(bits.position(), 8);
        assert!(bits.read(0).is_err());
        let mut bits = PacketBits::new(&[42]);
        assert!(bits.read(9).is_err());
        assert!(bits.read(1).is_err());
    }
    #[test]
    fn unordered_sparse_and_ordered_runs_preserve_entry_indices() {
        let prefix = [(0x564342, 24), (2, 16), (4, 24)];
        for (body, expected) in [
            (
                vec![(0, 1), (0, 1), (0, 5), (1, 5), (2, 5), (2, 5)],
                vec![1, 2, 3, 3],
            ),
            (
                vec![
                    (0, 1),
                    (1, 1),
                    (1, 1),
                    (0, 5),
                    (0, 1),
                    (1, 1),
                    (1, 5),
                    (1, 1),
                    (1, 5),
                ],
                vec![1, 0, 2, 2],
            ),
            (
                vec![(1, 1), (0, 5), (1, 3), (1, 2), (2, 2)],
                vec![1, 2, 3, 3],
            ),
            (vec![(1, 1), (0, 5), (0, 3), (4, 3)], vec![2, 2, 2, 2]),
        ] {
            let bytes = packed(&[prefix.to_vec(), body].concat());
            let book = CodewordLengths::parse(&mut PacketBits::new(&bytes), 4).unwrap();
            assert_eq!(book.dimensions, 2);
            assert_eq!(book.lengths, expected);
            assert!(CodewordLengths::parse(&mut PacketBits::new(&bytes), 3).is_err());
            for end in 0..bytes.len() {
                assert!(CodewordLengths::parse(&mut PacketBits::new(&bytes[..end]), 4).is_err());
            }
        }
    }
    #[test]
    fn malformed_ordered_run_and_unbounded_zero_runs_are_rejected() {
        for body in [vec![(1, 1), (0, 5), (5, 3)], vec![(1, 1), (31, 5), (0, 3)]] {
            let bytes = packed(&[vec![(0x564342, 24), (1, 16), (4, 24)], body].concat());
            assert!(CodewordLengths::parse(&mut PacketBits::new(&bytes), 4).is_err());
        }
    }
}

#[derive(Default)]
struct HuffmanNode {
    children: [Option<usize>; 2],
    entry: Option<usize>,
}
/// Scalar Vorbis entropy decoder. Entry order is significant, unlike a sorted
/// canonical Huffman assignment. Sparse indices are retained in decoded values.
pub struct HuffmanBook {
    nodes: Vec<HuffmanNode>,
    single: Option<usize>,
}
impl HuffmanBook {
    pub fn new(lengths: &[u8], max_nodes: usize) -> Result<Self> {
        let mut book = Self {
            nodes: Vec::new(),
            single: None,
        };
        let used = lengths.iter().filter(|&&n| n != 0).count();
        if used == 0 {
            return Err(invalid("empty Vorbis Huffman tree"));
        }
        if lengths.iter().any(|&n| n > 32) {
            return Err(invalid("Vorbis Huffman length exceeds 32"));
        }
        book.push(max_nodes)?;
        if used == 1 {
            let entry = lengths.iter().position(|&n| n != 0).unwrap();
            if lengths[entry] != 1 {
                return Err(invalid("single Vorbis entry must have length one"));
            }
            book.single = Some(entry);
            return Ok(book);
        }
        for (entry, &length) in lengths.iter().enumerate() {
            if length != 0 && !book.insert(0, length, entry, max_nodes)? {
                return Err(invalid("overspecified Vorbis Huffman tree"));
            }
        }
        if book
            .nodes
            .iter()
            .any(|n| n.entry.is_none() && n.children.iter().any(Option::is_none))
        {
            return Err(invalid("underspecified Vorbis Huffman tree"));
        }
        Ok(book)
    }
    fn push(&mut self, limit: usize) -> Result<usize> {
        if self.nodes.len() >= limit {
            return Err(invalid("Vorbis Huffman node budget exceeded"));
        }
        self.nodes
            .try_reserve_exact(1)
            .map_err(|_| invalid("cannot allocate Vorbis Huffman node"))?;
        let index = self.nodes.len();
        self.nodes.push(HuffmanNode::default());
        Ok(index)
    }
    fn insert(&mut self, index: usize, depth: u8, entry: usize, limit: usize) -> Result<bool> {
        if self.nodes[index].entry.is_some() {
            return Ok(false);
        }
        if depth == 0 {
            if self.nodes[index].children != [None, None] {
                return Ok(false);
            }
            self.nodes[index].entry = Some(entry);
            return Ok(true);
        }
        for side in 0..2 {
            let child = match self.nodes[index].children[side] {
                Some(child) => child,
                None => {
                    let child = self.push(limit)?;
                    self.nodes[index].children[side] = Some(child);
                    child
                }
            };
            if self.insert(child, depth - 1, entry, limit)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn decode(&self, bits: &mut PacketBits<'_>) -> Result<usize> {
        if let Some(entry) = self.single {
            bits.read(1)?;
            return Ok(entry);
        }
        let mut index = 0;
        loop {
            let node = &self.nodes[index];
            if let Some(entry) = node.entry {
                return Ok(entry);
            }
            index = node.children[bits.read(1)? as usize]
                .ok_or_else(|| invalid("missing Vorbis Huffman branch"))?;
        }
    }
}
#[cfg(test)]
mod huffman_tests {
    use super::*;
    fn stream(words: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut at = 0;
        for word in words {
            for bit in word.bytes() {
                if at % 8 == 0 {
                    bytes.push(0);
                }
                bytes[at / 8] |= (bit - b'0') << (at % 8);
                at += 1;
            }
        }
        bytes
    }
    #[test]
    fn xiph_unsorted_example_and_sparse_indices_decode_in_wire_order() {
        let lengths = [2, 4, 4, 4, 4, 2, 3, 3];
        let words = ["00", "0100", "0101", "0110", "0111", "10", "110", "111"];
        let book = HuffmanBook::new(&lengths, 15).unwrap();
        for order in [vec![0, 1, 2, 3, 4, 5, 6, 7], vec![7, 0, 5, 2, 6, 4, 1, 3]] {
            let bytes = stream(&order.iter().map(|&i| words[i]).collect::<Vec<_>>());
            let mut bits = PacketBits::new(&bytes);
            for entry in order {
                assert_eq!(book.decode(&mut bits).unwrap(), entry);
            }
        }
        let book = HuffmanBook::new(&[0, 1, 0, 2, 2, 0], 5).unwrap();
        let bytes = stream(&["0", "10", "11"]);
        let mut bits = PacketBits::new(&bytes);
        for entry in [1, 3, 4] {
            assert_eq!(book.decode(&mut bits).unwrap(), entry);
        }
    }
    #[test]
    fn malformed_trees_budgets_and_single_entry_erratum_are_enforced() {
        for lengths in [&[][..], &[0, 0], &[1, 1, 1], &[2, 2], &[0, 2, 0], &[33]] {
            assert!(HuffmanBook::new(lengths, 100).is_err());
        }
        assert!(HuffmanBook::new(&[1, 1], 2).is_err());
        let book = HuffmanBook::new(&[0, 0, 1, 0], 1).unwrap();
        for byte in [0, 255] {
            let bytes = [byte];
            let mut bits = PacketBits::new(&bytes);
            for _ in 0..8 {
                assert_eq!(book.decode(&mut bits).unwrap(), 2);
            }
            assert_eq!(bits.position(), 8);
            assert!(book.decode(&mut bits).is_err());
        }
    }
}

/// Vorbis codebook float representation is not IEEE 754.
pub fn unpack_float(word: u32) -> f32 {
    let mantissa = f64::from(word & 0x1f_ffff);
    let signed = if word & 0x8000_0000 != 0 {
        -mantissa
    } else {
        mantissa
    };
    (signed * 2f64.powi(((word >> 21) & 0x3ff) as i32 - 788)) as f32
}
/// Exact integer lattice root, avoiding floating-point rounding at boundaries.
pub fn lookup1_values(entries: u32, dimensions: u16) -> Result<u32> {
    if entries == 0 || dimensions == 0 {
        return Err(invalid("empty Vorbis lookup geometry"));
    }
    let fits = |base: u32| {
        let mut value = 1u32;
        for _ in 0..dimensions {
            let Some(next) = value.checked_mul(base) else {
                return false;
            };
            if next > entries {
                return false;
            }
            value = next;
        }
        true
    };
    let (mut low, mut high) = (1u32, entries);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if fits(mid) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    Ok(low)
}

pub struct VectorLookup {
    kind: u8,
    dimensions: u16,
    entries: usize,
    minimum: f32,
    delta: f32,
    sequence: bool,
    multiplicands: Vec<u16>,
}
impl VectorLookup {
    pub fn parse(
        bits: &mut PacketBits<'_>,
        dimensions: u16,
        entries: usize,
        max_values: usize,
    ) -> Result<Option<Self>> {
        let kind = bits.read(4)? as u8;
        if kind == 0 {
            return Ok(None);
        }
        if kind > 2 {
            return Err(invalid("reserved Vorbis lookup type"));
        }
        if dimensions == 0 || entries == 0 {
            return Err(invalid("empty Vorbis lookup geometry"));
        }
        let minimum = unpack_float(bits.read(32)?);
        let delta = unpack_float(bits.read(32)?);
        let width = bits.read(4)? as u8 + 1;
        let sequence = bits.read(1)? != 0;
        let count = if kind == 1 {
            lookup1_values(
                u32::try_from(entries).map_err(|_| invalid("Vorbis entry count overflow"))?,
                dimensions,
            )? as usize
        } else {
            entries
                .checked_mul(usize::from(dimensions))
                .ok_or_else(|| invalid("Vorbis lookup size overflow"))?
        };
        if count > max_values {
            return Err(invalid("Vorbis lookup exceeds value budget"));
        }
        let mut multiplicands = Vec::new();
        multiplicands
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate Vorbis lookup"))?;
        for _ in 0..count {
            multiplicands.push(bits.read(width)? as u16);
        }
        Ok(Some(Self {
            kind,
            dimensions,
            entries,
            minimum,
            delta,
            sequence,
            multiplicands,
        }))
    }
    /// Fills caller storage, without expanding every codebook vector at setup.
    pub fn vector(&self, entry: usize, output: &mut [f32]) -> Result<()> {
        if entry >= self.entries || output.len() != usize::from(self.dimensions) {
            return Err(invalid("invalid Vorbis lookup vector geometry"));
        }
        let mut last = 0.;
        let mut quotient = entry;
        for (i, value) in output.iter_mut().enumerate() {
            let offset = if self.kind == 1 {
                let digit = quotient % self.multiplicands.len();
                quotient /= self.multiplicands.len();
                digit
            } else {
                entry * usize::from(self.dimensions) + i
            };
            *value = f32::from(self.multiplicands[offset]) * self.delta + self.minimum + last;
            if self.sequence {
                last = *value;
            }
        }
        Ok(())
    }
}
/// A complete packed book, ready for scalar or vector entropy decode.
pub struct Codebook {
    pub dimensions: u16,
    entries: usize,
    huffman: HuffmanBook,
    lookup: Option<VectorLookup>,
}
impl Codebook {
    pub fn parse(
        bits: &mut PacketBits<'_>,
        max_entries: usize,
        max_nodes: usize,
        max_values: usize,
    ) -> Result<Self> {
        let lengths = CodewordLengths::parse(bits, max_entries)?;
        let huffman = HuffmanBook::new(&lengths.lengths, max_nodes)?;
        let lookup =
            VectorLookup::parse(bits, lengths.dimensions, lengths.lengths.len(), max_values)?;
        Ok(Self {
            dimensions: lengths.dimensions,
            entries: lengths.lengths.len(),
            huffman,
            lookup,
        })
    }
    pub fn scalar(&self, bits: &mut PacketBits<'_>) -> Result<usize> {
        self.huffman.decode(bits)
    }
    pub fn vector(&self, bits: &mut PacketBits<'_>, output: &mut [f32]) -> Result<()> {
        let lookup = self
            .lookup
            .as_ref()
            .ok_or_else(|| invalid("Vorbis scalar-only book used for vector decode"))?;
        if output.len() != usize::from(self.dimensions) {
            return Err(invalid("invalid Vorbis vector output length"));
        }
        lookup.vector(self.scalar(bits)?, output)
    }
}
#[cfg(test)]
mod lookup_tests {
    use super::*;
    fn packed(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut bit = 0;
        for &(value, width) in fields {
            for shift in 0..width {
                if bit % 8 == 0 {
                    data.push(0);
                }
                data[bit / 8] |= (((value >> shift) & 1) as u8) << (bit % 8);
                bit += 1;
            }
        }
        data
    }
    #[test]
    fn lattice_roots_are_exact_at_power_boundaries() {
        for dimensions in 1..=12u16 {
            for entries in 1..=1024u32 {
                let root = lookup1_values(entries, dimensions).unwrap();
                assert!(
                    u128::from(root)
                        .checked_pow(u32::from(dimensions))
                        .is_some_and(|p| p <= u128::from(entries))
                );
                assert!(
                    u128::from(root + 1)
                        .checked_pow(u32::from(dimensions))
                        .is_none_or(|p| p > u128::from(entries))
                );
            }
        }
        assert_eq!(lookup1_values(u32::MAX, 1).unwrap(), u32::MAX);
        assert_eq!(lookup1_values(0xffffff, 65535).unwrap(), 1);
        assert_eq!(unpack_float(788 << 21 | 3), 3.);
        assert_eq!(unpack_float(0x80000000 | 787 << 21 | 3), -1.5);
    }
    #[test]
    fn both_lookup_types_and_sequence_match_authored_vectors() {
        for kind in [1, 2] {
            for sequence in [false, true] {
                let mut fields = vec![
                    (kind, 4),
                    (0x80000000 | 788 << 21 | 1, 32),
                    (787 << 21 | 1, 32),
                    (2, 4),
                    (u32::from(sequence), 1),
                ];
                let values: Vec<_> = if kind == 1 {
                    vec![2, 6]
                } else {
                    vec![2, 6, 6, 2, 2, 2, 6, 6]
                };
                fields.extend(values.iter().map(|&v| (v, 3)));
                let bytes = packed(&fields);
                let lookup = VectorLookup::parse(&mut PacketBits::new(&bytes), 2, 4, 8)
                    .unwrap()
                    .unwrap();
                for entry in 0..4 {
                    let offsets = if kind == 1 {
                        [entry % 2, entry / 2]
                    } else {
                        [2 * entry, 2 * entry + 1]
                    };
                    let a = values[offsets[0]] as f32 * 0.5 - 1.;
                    let b = values[offsets[1]] as f32 * 0.5 - 1. + if sequence { a } else { 0. };
                    let mut actual = [0.; 2];
                    lookup.vector(entry, &mut actual).unwrap();
                    assert_eq!(actual, [a, b]);
                }
                assert!(VectorLookup::parse(&mut PacketBits::new(&bytes), 2, 4, 1).is_err());
                for end in 0..bytes.len() {
                    assert!(
                        VectorLookup::parse(&mut PacketBits::new(&bytes[..end]), 2, 4, 8).is_err()
                    );
                }
            }
        }
    }
    #[test]
    fn complete_book_combines_entropy_and_vectors_and_refuses_scalar_lookup() {
        for kind in [0, 1] {
            let mut fields = vec![
                (0x564342, 24),
                (2, 16),
                (4, 24),
                (1, 1),
                (1, 5),
                (4, 3),
                (kind, 4),
            ];
            if kind == 1 {
                fields.extend([(0, 32), (788 << 21 | 1, 32), (2, 4), (0, 1), (2, 3), (6, 3)]);
            }
            let bytes = packed(&fields);
            let book = Codebook::parse(&mut PacketBits::new(&bytes), 4, 7, 2).unwrap();
            let wire = packed(&[(0b11100100, 8)]);
            let mut bits = PacketBits::new(&wire);
            // LSB packet order encodes 00, 10, 01, 11.
            for entry in [0, 2, 1, 3] {
                assert_eq!(book.scalar(&mut bits).unwrap(), entry);
            }
            let mut bits = PacketBits::new(&[0]);
            let mut output = [99.; 2];
            if kind == 0 {
                assert!(book.vector(&mut bits, &mut output).is_err());
                assert_eq!(bits.position(), 0);
            } else {
                book.vector(&mut bits, &mut output).unwrap();
                assert_eq!(output, [2., 2.]);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Submap {
    pub floor: u8,
    pub residue: u8,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Mapping {
    pub coupling: Vec<(u8, u8)>,
    pub mux: Vec<u8>,
    pub submaps: Vec<Submap>,
}
impl Mapping {
    pub fn parse(
        bits: &mut PacketBits<'_>,
        channels: u8,
        floors: usize,
        residues: usize,
    ) -> Result<Self> {
        if channels == 0 {
            return Err(invalid("zero Vorbis mapping channels"));
        }
        if bits.read(16)? != 0 {
            return Err(invalid("reserved Vorbis mapping type"));
        }
        let submaps = if bits.read(1)? != 0 {
            bits.read(4)? as usize + 1
        } else {
            1
        };
        let steps = if bits.read(1)? != 0 {
            bits.read(8)? as usize + 1
        } else {
            0
        };
        let width = (u8::BITS - (channels - 1).leading_zeros()) as u8;
        let mut coupling = Vec::new();
        coupling
            .try_reserve_exact(steps)
            .map_err(|_| invalid("cannot allocate Vorbis coupling"))?;
        for _ in 0..steps {
            let magnitude = bits.read(width)? as u8;
            let angle = bits.read(width)? as u8;
            if magnitude == angle || magnitude >= channels || angle >= channels {
                return Err(invalid("invalid Vorbis coupling channels"));
            }
            coupling.push((magnitude, angle));
        }
        if bits.read(2)? != 0 {
            return Err(invalid("nonzero Vorbis mapping reserved bits"));
        }
        let mut mux = Vec::new();
        mux.try_reserve_exact(usize::from(channels))
            .map_err(|_| invalid("cannot allocate Vorbis mux"))?;
        for _ in 0..channels {
            let index = if submaps > 1 { bits.read(4)? as u8 } else { 0 };
            if usize::from(index) >= submaps {
                return Err(invalid("Vorbis mux exceeds submap count"));
            }
            mux.push(index);
        }
        let mut configs = Vec::new();
        configs
            .try_reserve_exact(submaps)
            .map_err(|_| invalid("cannot allocate Vorbis submaps"))?;
        for _ in 0..submaps {
            bits.read(8)?; // Unused time placeholder, not a reserved-zero field.
            let floor = bits.read(8)? as u8;
            let residue = bits.read(8)? as u8;
            if usize::from(floor) >= floors || usize::from(residue) >= residues {
                return Err(invalid("Vorbis submap reference exceeds configured count"));
            }
            configs.push(Submap { floor, residue });
        }
        Ok(Self {
            coupling,
            mux,
            submaps: configs,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub long: bool,
    pub mapping: u8,
}
impl Mode {
    pub fn parse(bits: &mut PacketBits<'_>, mappings: usize) -> Result<Self> {
        let long = bits.read(1)? != 0;
        if bits.read(16)? != 0 || bits.read(16)? != 0 {
            return Err(invalid("reserved Vorbis window or transform type"));
        }
        let mapping = bits.read(8)? as u8;
        if usize::from(mapping) >= mappings {
            return Err(invalid("Vorbis mode mapping exceeds configured count"));
        }
        Ok(Self { long, mapping })
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct PacketMode {
    pub mode: usize,
    pub previous_long: bool,
    pub next_long: bool,
}
impl PacketMode {
    pub fn parse(bits: &mut PacketBits<'_>, modes: &[Mode]) -> Result<Self> {
        if modes.is_empty() || modes.len() > 64 {
            return Err(invalid("invalid Vorbis mode count"));
        }
        if bits.read(1)? != 0 {
            return Err(invalid("non-audio Vorbis packet"));
        }
        let width = (usize::BITS - (modes.len() - 1).leading_zeros()) as u8;
        let mode = bits.read(width)? as usize;
        let config = modes
            .get(mode)
            .ok_or_else(|| invalid("Vorbis packet mode exceeds configured count"))?;
        let (previous_long, next_long) = if config.long {
            (bits.read(1)? != 0, bits.read(1)? != 0)
        } else {
            (false, false)
        };
        Ok(Self {
            mode,
            previous_long,
            next_long,
        })
    }
}
#[cfg(test)]
mod routing_tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        for &(v, n) in fields {
            for i in 0..n {
                if at % 8 == 0 {
                    out.push(0);
                }
                out[at / 8] |= (((v >> i) & 1) as u8) << (at % 8);
                at += 1;
            }
        }
        out
    }
    #[test]
    fn mapping_supports_all_channel_counts_and_checks_routes() {
        for channels in 1..=255 {
            let bytes = pack(&[(0, 16), (0, 1), (0, 1), (0, 2), (255, 8), (0, 8), (0, 8)]);
            let mapping = Mapping::parse(&mut PacketBits::new(&bytes), channels, 1, 1).unwrap();
            assert_eq!(mapping.mux, vec![0; usize::from(channels)]);
            assert!(mapping.coupling.is_empty());
            for end in 0..bytes.len() {
                assert!(
                    Mapping::parse(&mut PacketBits::new(&bytes[..end]), channels, 1, 1).is_err()
                );
            }
        }
        let bytes = pack(&[
            (0, 16),
            (1, 1),
            (1, 4),
            (1, 1),
            (0, 8),
            (0, 2),
            (2, 2),
            (0, 2),
            (0, 4),
            (1, 4),
            (1, 4),
            (0, 8),
            (0, 8),
            (1, 8),
            (7, 8),
            (1, 8),
            (0, 8),
        ]);
        let mapping = Mapping::parse(&mut PacketBits::new(&bytes), 3, 2, 2).unwrap();
        assert_eq!(mapping.coupling, vec![(0, 2)]);
        assert_eq!(mapping.mux, vec![0, 1, 1]);
        assert_eq!(
            mapping.submaps[1],
            Submap {
                floor: 1,
                residue: 0
            }
        );
        assert!(Mapping::parse(&mut PacketBits::new(&bytes), 3, 1, 2).is_err());
        assert!(Mapping::parse(&mut PacketBits::new(&bytes), 3, 2, 1).is_err());
        for (m, a) in [(0, 0), (3, 1), (1, 3)] {
            let bytes = pack(&[(0, 16), (0, 1), (1, 1), (0, 8), (m, 2), (a, 2)]);
            assert!(Mapping::parse(&mut PacketBits::new(&bytes), 3, 1, 1).is_err());
        }
    }
    #[test]
    fn packet_mode_uses_mode_count_instead_of_assuming_one_flag_bit() {
        let modes = [
            Mode {
                long: false,
                mapping: 0,
            },
            Mode {
                long: false,
                mapping: 0,
            },
            Mode {
                long: true,
                mapping: 1,
            },
        ];
        for previous in [false, true] {
            for next in [false, true] {
                let bytes = pack(&[
                    (0, 1),
                    (2, 2),
                    (u32::from(previous), 1),
                    (u32::from(next), 1),
                ]);
                let mut bits = PacketBits::new(&bytes);
                assert_eq!(
                    PacketMode::parse(&mut bits, &modes).unwrap(),
                    PacketMode {
                        mode: 2,
                        previous_long: previous,
                        next_long: next
                    }
                );
                assert_eq!(bits.position(), 5);
            }
        }
        let bytes = pack(&[(0, 1), (3, 2)]);
        assert!(PacketMode::parse(&mut PacketBits::new(&bytes), &modes).is_err());
        let bytes = pack(&[(0, 1), (1, 2)]);
        let mut bits = PacketBits::new(&bytes);
        assert_eq!(PacketMode::parse(&mut bits, &modes).unwrap().mode, 1);
        assert_eq!(bits.position(), 3);
        let bytes = pack(&[(1, 1), (0, 16), (0, 16), (1, 8)]);
        assert_eq!(
            Mode::parse(&mut PacketBits::new(&bytes), 2).unwrap(),
            modes[2]
        );
        assert!(Mode::parse(&mut PacketBits::new(&bytes), 1).is_err());
    }
}

impl Mapping {
    fn validate_coupling(&self, channels: usize) -> Result<()> {
        if channels != self.mux.len()
            || self
                .coupling
                .iter()
                .any(|&(m, a)| m == a || usize::from(m) >= channels || usize::from(a) >= channels)
        {
            return Err(invalid("invalid Vorbis coupling geometry"));
        }
        Ok(())
    }
    /// Propagate residue participation in coding order. Floor presence itself
    /// must remain separately recorded for later floor synthesis.
    pub fn propagate_nonzero(&self, nonzero: &mut [bool]) -> Result<()> {
        self.validate_coupling(nonzero.len())?;
        for &(m, a) in &self.coupling {
            let (m, a) = (usize::from(m), usize::from(a));
            if nonzero[m] || nonzero[a] {
                nonzero[m] = true;
                nonzero[a] = true;
            }
        }
        Ok(())
    }
    /// Undo square-polar coupling in reverse coding order, in place.
    /// Validate all channel geometry before changing any sample.
    pub fn inverse_coupling(&self, spectra: &mut [Vec<f32>]) -> Result<()> {
        self.validate_coupling(spectra.len())?;
        let size = spectra.first().map_or(0, Vec::len);
        if spectra.iter().any(|s| s.len() != size) {
            return Err(invalid("unequal Vorbis spectral vector lengths"));
        }
        for &(m, a) in self.coupling.iter().rev() {
            let (m, a) = (usize::from(m), usize::from(a));
            let (magnitude, angle) = if m < a {
                let (left, right) = spectra.split_at_mut(a);
                (&mut left[m], &mut right[0])
            } else {
                let (left, right) = spectra.split_at_mut(m);
                (&mut right[0], &mut left[a])
            };
            for (m, a) in magnitude.iter_mut().zip(angle.iter_mut()) {
                let (new_m, new_a) = if *m > 0. {
                    if *a > 0. {
                        (*m, *m - *a)
                    } else {
                        (*m + *a, *m)
                    }
                } else if *a > 0. {
                    (*m, *m + *a)
                } else {
                    (*m - *a, *m)
                };
                *m = new_m;
                *a = new_a;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod coupling_tests {
    use super::*;
    fn mapping(pairs: Vec<(u8, u8)>, channels: usize) -> Mapping {
        Mapping {
            coupling: pairs,
            mux: vec![0; channels],
            submaps: vec![Submap {
                floor: 0,
                residue: 0,
            }],
        }
    }
    #[test]
    fn inverse_coupling_covers_quadrants_and_zero_boundaries_in_either_channel_order() {
        let magnitude = vec![3., 3., -3., -3., 0., 0., 3., -3.];
        let angle = vec![2., -2., 2., -2., 2., -2., 0., 0.];
        let expected_m = vec![3., 1., -3., -1., 0., 2., 3., -3.];
        let expected_a = vec![1., 3., -1., -3., 2., 0., 3., -3.];
        for swapped in [false, true] {
            let mut spectra = if swapped {
                vec![angle.clone(), magnitude.clone()]
            } else {
                vec![magnitude.clone(), angle.clone()]
            };
            mapping(if swapped { vec![(1, 0)] } else { vec![(0, 1)] }, 2)
                .inverse_coupling(&mut spectra)
                .unwrap();
            assert_eq!(
                spectra,
                if swapped {
                    vec![expected_a.clone(), expected_m.clone()]
                } else {
                    vec![expected_m.clone(), expected_a.clone()]
                }
            );
        }
    }
    #[test]
    fn coupling_steps_reverse_but_nonzero_propagates_forward_and_failure_is_atomic() {
        let config = mapping(vec![(0, 1), (1, 2)], 3);
        let mut spectra = vec![vec![3.], vec![2.], vec![1.]];
        config.inverse_coupling(&mut spectra).unwrap();
        assert_eq!(spectra, vec![vec![3.], vec![1.], vec![1.]]);
        let mut nonzero = [true, false, false];
        config.propagate_nonzero(&mut nonzero).unwrap();
        assert_eq!(nonzero, [true, true, true]);
        let mut nonzero = [false, false, true];
        config.propagate_nonzero(&mut nonzero).unwrap();
        assert_eq!(nonzero, [false, true, true]);
        let mut spectra = vec![vec![3.], vec![2.], vec![1., 7.]];
        let before = spectra.clone();
        assert!(config.inverse_coupling(&mut spectra).is_err());
        assert_eq!(spectra, before);
        let invalid = mapping(vec![(0, 1), (2, 9)], 3);
        let mut flags = [true, false, false];
        assert!(invalid.propagate_nonzero(&mut flags).is_err());
        assert_eq!(flags, [true, false, false]);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Residue {
    pub kind: u16,
    pub begin: u32,
    pub end: u32,
    pub partition_size: u32,
    pub classbook: u8,
    pub books: Vec<[Option<u8>; 8]>,
}
impl Residue {
    /// Parse setup and qualify every referenced vector codebook before packets.
    pub fn parse(bits: &mut PacketBits<'_>, codebooks: &[Codebook]) -> Result<Self> {
        let kind = bits.read(16)? as u16;
        if kind > 2 {
            return Err(invalid("reserved Vorbis residue type"));
        }
        let begin = bits.read(24)?;
        let end = bits.read(24)?;
        let partition_size = bits.read(24)? + 1;
        let classifications = bits.read(6)? as usize + 1;
        let classbook = bits.read(8)? as u8;
        let class = codebooks
            .get(usize::from(classbook))
            .ok_or_else(|| invalid("Vorbis residue classbook exceeds configured count"))?;
        if classifications
            .checked_pow(u32::from(class.dimensions))
            .is_none_or(|n| n > class.entries)
        {
            return Err(invalid(
                "Vorbis residue classification space exceeds classbook entries",
            ));
        }
        let mut cascade = [0u8; 64];
        for mask in &mut cascade[..classifications] {
            let low = bits.read(3)? as u8;
            let high = if bits.read(1)? != 0 {
                bits.read(5)? as u8
            } else {
                0
            };
            *mask = low | (high << 3);
        }
        let mut books = Vec::new();
        books
            .try_reserve_exact(classifications)
            .map_err(|_| invalid("cannot allocate Vorbis residue books"))?;
        for &mask in &cascade[..classifications] {
            let mut passes = [None; 8];
            for (pass, slot) in passes.iter_mut().enumerate() {
                if mask & (1 << pass) != 0 {
                    let index = bits.read(8)? as u8;
                    let book = codebooks.get(usize::from(index)).ok_or_else(|| {
                        invalid("Vorbis residue vector book exceeds configured count")
                    })?;
                    if book.lookup.is_none() {
                        return Err(invalid("Vorbis residue book has no vector lookup"));
                    }
                    *slot = Some(index);
                }
            }
            books.push(passes);
        }
        Ok(Self {
            kind,
            begin,
            end,
            partition_size,
            classbook,
            books,
        })
    }
}
#[cfg(test)]
mod residue_tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        for &(v, n) in fields {
            for i in 0..n {
                if at % 8 == 0 {
                    out.push(0);
                }
                out[at / 8] |= (((v >> i) & 1) as u8) << (at % 8);
                at += 1;
            }
        }
        out
    }
    fn books() -> Vec<Codebook> {
        let mut result = Vec::new();
        for kind in [0, 1] {
            let mut fields = vec![
                (0x564342, 24),
                (1, 16),
                (2, 24),
                (1, 1),
                (0, 5),
                (2, 2),
                (kind, 4),
            ];
            if kind == 1 {
                fields.extend([(0, 32), (788 << 21 | 1, 32), (0, 4), (0, 1), (0, 1), (1, 1)]);
            }
            let bytes = pack(&fields);
            result.push(Codebook::parse(&mut PacketBits::new(&bytes), 2, 3, 2).unwrap());
        }
        result
    }
    #[test]
    fn all_residue_types_preserve_sparse_eight_pass_cascades_and_truncations() {
        let books = books();
        for kind in 0..=2 {
            let bytes = pack(&[
                (kind, 16),
                (7, 24),
                (103, 24),
                (15, 24),
                (1, 6),
                (0, 8),
                (1, 3),
                (1, 1),
                (16, 5),
                (2, 3),
                (0, 1),
                (1, 8),
                (1, 8),
                (1, 8),
            ]);
            let residue = Residue::parse(&mut PacketBits::new(&bytes), &books).unwrap();
            assert_eq!(residue.kind, kind as u16);
            assert_eq!(residue.begin, 7);
            assert_eq!(residue.end, 103);
            assert_eq!(residue.partition_size, 16);
            assert_eq!(
                residue.books[0],
                [Some(1), None, None, None, None, None, None, Some(1)]
            );
            assert_eq!(
                residue.books[1],
                [None, Some(1), None, None, None, None, None, None]
            );
            for end in 0..bytes.len() {
                assert!(Residue::parse(&mut PacketBits::new(&bytes[..end]), &books).is_err());
            }
        }
    }
    #[test]
    fn residue_rejects_reserved_types_bad_classification_and_scalar_vector_books() {
        let books = books();
        for (kind, classes, classbook, vector) in [
            (3, 1, 0, 1),
            (0, 2, 0, 1),
            (0, 1, 9, 1),
            (0, 1, 0, 0),
            (0, 1, 0, 9),
        ] {
            let bytes = pack(&[
                (kind, 16),
                (0, 24),
                (16, 24),
                (15, 24),
                (classes, 6),
                (classbook, 8),
                (1, 3),
                (0, 1),
                (0, 3),
                (0, 1),
                (vector, 8),
            ]);
            assert!(Residue::parse(&mut PacketBits::new(&bytes), &books).is_err());
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Floor0 {
    pub order: u8,
    pub rate: u16,
    pub bark_map_size: u16,
    pub amplitude_bits: u8,
    pub amplitude_offset: u8,
    pub books: Vec<u8>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct FloorClass {
    pub dimensions: u8,
    pub subclasses: u8,
    pub masterbook: Option<u8>,
    pub books: Vec<Option<u8>>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Floor1 {
    pub partitions: Vec<u8>,
    pub classes: Vec<FloorClass>,
    pub multiplier: u8,
    pub x: Vec<u16>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Floor {
    Zero(Floor0),
    One(Floor1),
}
fn floor_book(index: u8, count: usize) -> Result<u8> {
    if usize::from(index) >= count {
        return Err(invalid("Vorbis floor book exceeds configured count"));
    }
    Ok(index)
}
impl Floor {
    pub fn parse(bits: &mut PacketBits<'_>, codebooks: usize) -> Result<Self> {
        match bits.read(16)? {
            0 => {
                let order = bits.read(8)? as u8;
                let rate = bits.read(16)? as u16;
                let bark_map_size = bits.read(16)? as u16;
                let amplitude_bits = bits.read(6)? as u8;
                let amplitude_offset = bits.read(8)? as u8;
                let count = bits.read(4)? as usize + 1;
                let mut books = Vec::new();
                books
                    .try_reserve_exact(count)
                    .map_err(|_| invalid("cannot allocate Vorbis floor books"))?;
                for _ in 0..count {
                    books.push(floor_book(bits.read(8)? as u8, codebooks)?);
                }
                Ok(Self::Zero(Floor0 {
                    order,
                    rate,
                    bark_map_size,
                    amplitude_bits,
                    amplitude_offset,
                    books,
                }))
            }
            1 => {
                let count = bits.read(5)? as usize;
                let mut partitions = Vec::new();
                partitions
                    .try_reserve_exact(count)
                    .map_err(|_| invalid("cannot allocate Vorbis floor partitions"))?;
                for _ in 0..count {
                    partitions.push(bits.read(4)? as u8);
                }
                let classes_count = partitions.iter().max().map_or(0, |&n| usize::from(n) + 1);
                let mut classes = Vec::new();
                classes
                    .try_reserve_exact(classes_count)
                    .map_err(|_| invalid("cannot allocate Vorbis floor classes"))?;
                for _ in 0..classes_count {
                    let dimensions = bits.read(3)? as u8 + 1;
                    let subclasses = bits.read(2)? as u8;
                    let masterbook = if subclasses > 0 {
                        Some(floor_book(bits.read(8)? as u8, codebooks)?)
                    } else {
                        None
                    };
                    let mut books = Vec::new();
                    books
                        .try_reserve_exact(1 << subclasses)
                        .map_err(|_| invalid("cannot allocate Vorbis floor subclass books"))?;
                    for _ in 0..1usize << subclasses {
                        let encoded = bits.read(8)? as u8;
                        books.push(if encoded == 0 {
                            None
                        } else {
                            Some(floor_book(encoded - 1, codebooks)?)
                        });
                    }
                    classes.push(FloorClass {
                        dimensions,
                        subclasses,
                        masterbook,
                        books,
                    });
                }
                let multiplier = bits.read(2)? as u8 + 1;
                let rangebits = bits.read(4)? as u8;
                let values = 2 + partitions
                    .iter()
                    .map(|&class| usize::from(classes[usize::from(class)].dimensions))
                    .sum::<usize>();
                if values > 65 {
                    return Err(invalid("Vorbis floor exceeds 65 X positions"));
                }
                let mut x = Vec::new();
                x.try_reserve_exact(values)
                    .map_err(|_| invalid("cannot allocate Vorbis floor X positions"))?;
                x.extend([0, 1u16 << rangebits]);
                for _ in 2..values {
                    let value = bits.read(rangebits)? as u16;
                    if x.contains(&value) {
                        return Err(invalid("duplicate Vorbis floor X position"));
                    }
                    x.push(value);
                }
                Ok(Self::One(Floor1 {
                    partitions,
                    classes,
                    multiplier,
                    x,
                }))
            }
            _ => Err(invalid("reserved Vorbis floor type")),
        }
    }
}
#[cfg(test)]
mod floor_setup_tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        for &(v, n) in fields {
            for i in 0..n {
                if at % 8 == 0 {
                    out.push(0);
                }
                out[at / 8] |= (((v >> i) & 1) as u8) << (at % 8);
                at += 1;
            }
        }
        out
    }
    #[test]
    fn floor_zero_and_one_setup_preserve_configuration_and_check_truncations() {
        let zero = pack(&[
            (0, 16),
            (10, 8),
            (48000, 16),
            (64, 16),
            (6, 6),
            (120, 8),
            (1, 4),
            (0, 8),
            (1, 8),
        ]);
        assert_eq!(
            Floor::parse(&mut PacketBits::new(&zero), 2).unwrap(),
            Floor::Zero(Floor0 {
                order: 10,
                rate: 48000,
                bark_map_size: 64,
                amplitude_bits: 6,
                amplitude_offset: 120,
                books: vec![0, 1]
            })
        );
        let one = pack(&[
            (1, 16),
            (2, 5),
            (0, 4),
            (0, 4),
            (1, 3),
            (1, 2),
            (1, 8),
            (0, 8),
            (1, 8),
            (2, 2),
            (7, 4),
            (64, 7),
            (32, 7),
            (96, 7),
            (16, 7),
        ]);
        assert_eq!(
            Floor::parse(&mut PacketBits::new(&one), 2).unwrap(),
            Floor::One(Floor1 {
                partitions: vec![0, 0],
                classes: vec![FloorClass {
                    dimensions: 2,
                    subclasses: 1,
                    masterbook: Some(1),
                    books: vec![None, Some(0)]
                }],
                multiplier: 3,
                x: vec![0, 128, 64, 32, 96, 16]
            })
        );
        for bytes in [&zero, &one] {
            for end in 0..bytes.len() {
                assert!(Floor::parse(&mut PacketBits::new(&bytes[..end]), 2).is_err());
            }
        }
        let empty = pack(&[(1, 16), (0, 5), (0, 2), (0, 4)]);
        assert_eq!(
            Floor::parse(&mut PacketBits::new(&empty), 0).unwrap(),
            Floor::One(Floor1 {
                partitions: vec![],
                classes: vec![],
                multiplier: 1,
                x: vec![0, 1]
            })
        );
    }
    #[test]
    fn floor_setup_rejects_duplicate_positions_bad_books_and_excess_values() {
        for book in [2, 255] {
            let bytes = pack(&[
                (0, 16),
                (10, 8),
                (48000, 16),
                (64, 16),
                (6, 6),
                (120, 8),
                (0, 4),
                (book, 8),
            ]);
            assert!(Floor::parse(&mut PacketBits::new(&bytes), 2).is_err());
        }
        for x in [0, 64] {
            let bytes = pack(&[
                (1, 16),
                (1, 5),
                (0, 4),
                (1, 3),
                (0, 2),
                (0, 8),
                (0, 2),
                (7, 4),
                (64, 7),
                (x, 7),
            ]);
            assert!(Floor::parse(&mut PacketBits::new(&bytes), 2).is_err());
        }
        let mut fields = vec![(1, 16), (8, 5)];
        fields.extend([(0, 4); 8]);
        fields.extend([(7, 3), (0, 2), (0, 8), (0, 2), (7, 4)]);
        assert!(Floor::parse(&mut PacketBits::new(&pack(&fields)), 2).is_err());
    }
}

/// Aggregate book storage admission, supplied by the caller for the whole setup.
#[derive(Clone, Copy)]
pub struct SetupLimits {
    pub entries: usize,
    pub nodes: usize,
    pub multiplicands: usize,
}
pub struct Setup {
    pub codebooks: Vec<Codebook>,
    pub floors: Vec<Floor>,
    pub residues: Vec<Residue>,
    pub mappings: Vec<Mapping>,
    pub modes: Vec<Mode>,
}
fn reserve<T>(count: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| invalid("cannot allocate Vorbis setup list"))?;
    Ok(values)
}
impl Setup {
    pub fn parse(packet: &[u8], channels: u8, mut limits: SetupLimits) -> Result<Self> {
        if channels == 0 {
            return Err(invalid("zero Vorbis setup channels"));
        }
        let mut bits = PacketBits::new(packet);
        if bits.read(8)? != 5 {
            return Err(invalid("Vorbis setup packet type is not five"));
        }
        for byte in b"vorbis" {
            if bits.read(8)? != u32::from(*byte) {
                return Err(invalid("invalid Vorbis setup signature"));
            }
        }
        let count = bits.read(8)? as usize + 1;
        let mut codebooks = reserve(count)?;
        for _ in 0..count {
            let book = Codebook::parse(
                &mut bits,
                limits.entries,
                limits.nodes,
                limits.multiplicands,
            )?;
            limits.entries -= book.entries;
            limits.nodes -= book.huffman.nodes.len();
            limits.multiplicands -= book.lookup.as_ref().map_or(0, |v| v.multiplicands.len());
            codebooks.push(book);
        }
        let times = bits.read(6)? + 1;
        for _ in 0..times {
            if bits.read(16)? != 0 {
                return Err(invalid("nonzero Vorbis time transform"));
            }
        }
        let count = bits.read(6)? as usize + 1;
        let mut floors = reserve(count)?;
        for _ in 0..count {
            floors.push(Floor::parse(&mut bits, codebooks.len())?);
        }
        // Floor zero uses its listed books in vector context.
        for floor in &floors {
            if let Floor::Zero(config) = floor {
                if config
                    .books
                    .iter()
                    .any(|&b| codebooks[usize::from(b)].lookup.is_none())
                {
                    return Err(invalid("Vorbis floor zero book has no vector lookup"));
                }
            }
        }
        let count = bits.read(6)? as usize + 1;
        let mut residues = reserve(count)?;
        for _ in 0..count {
            residues.push(Residue::parse(&mut bits, &codebooks)?);
        }
        let count = bits.read(6)? as usize + 1;
        let mut mappings = reserve(count)?;
        for _ in 0..count {
            mappings.push(Mapping::parse(
                &mut bits,
                channels,
                floors.len(),
                residues.len(),
            )?);
        }
        let count = bits.read(6)? as usize + 1;
        let mut modes = reserve(count)?;
        for _ in 0..count {
            modes.push(Mode::parse(&mut bits, mappings.len())?);
        }
        if bits.read(1)? != 1 {
            return Err(invalid("missing Vorbis setup framing bit"));
        }
        Ok(Self {
            codebooks,
            floors,
            residues,
            mappings,
            modes,
        })
    }
}
#[cfg(test)]
mod setup_tests {
    use super::*;
    fn fixture(books: usize, framing: u32, time: u32) -> Vec<u8> {
        let mut fields = vec![(5, 8)];
        fields.extend(b"vorbis".iter().map(|&b| (u32::from(b), 8)));
        fields.push(((books - 1) as u32, 8));
        for _ in 0..books {
            fields.extend([
                (0x564342, 24),
                (1, 16),
                (1, 24),
                (0, 1),
                (0, 1),
                (0, 5),
                (0, 4),
            ]);
        }
        fields.extend([
            (0, 6),
            (time, 16), // time
            (0, 6),
            (1, 16),
            (0, 5),
            (0, 2),
            (6, 4), // floor 1
            (0, 6),
            (0, 16),
            (0, 24),
            (32, 24),
            (15, 24),
            (0, 6),
            (0, 8),
            (0, 3),
            (0, 1), // residue
            (0, 6),
            (0, 16),
            (0, 1),
            (0, 1),
            (0, 2),
            (0, 8),
            (0, 8),
            (0, 8), // mapping
            (0, 6),
            (0, 1),
            (0, 16),
            (0, 16),
            (0, 8),
            (framing, 1),
        ]);
        let mut out = Vec::new();
        let mut at = 0;
        for (v, n) in fields {
            for i in 0..n {
                if at % 8 == 0 {
                    out.push(0);
                }
                out[at / 8] |= (((v >> i) & 1) as u8) << (at % 8);
                at += 1;
            }
        }
        out
    }
    #[test]
    fn complete_setup_connects_sections_and_admits_aggregate_storage() {
        let bytes = fixture(2, 1, 0);
        let limits = SetupLimits {
            entries: 2,
            nodes: 2,
            multiplicands: 0,
        };
        let setup = Setup::parse(&bytes, 2, limits).unwrap();
        assert_eq!(setup.codebooks.len(), 2);
        assert_eq!(setup.floors.len(), 1);
        assert_eq!(setup.residues.len(), 1);
        assert_eq!(setup.mappings.len(), 1);
        assert_eq!(
            setup.modes,
            vec![Mode {
                long: false,
                mapping: 0
            }]
        );
        assert!(
            Setup::parse(
                &bytes,
                2,
                SetupLimits {
                    entries: 1,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(Setup::parse(&bytes, 2, SetupLimits { nodes: 1, ..limits }).is_err());
        for end in 0..bytes.len() {
            assert!(Setup::parse(&bytes[..end], 2, limits).is_err());
        }
        assert!(Setup::parse(&fixture(1, 0, 0), 2, limits).is_err());
        assert!(Setup::parse(&fixture(1, 1, 1), 2, limits).is_err());
        let mut invalid = bytes.clone();
        invalid[0] = 3;
        assert!(Setup::parse(&invalid, 2, limits).is_err());
        invalid = bytes;
        invalid[1] = 0;
        assert!(Setup::parse(&invalid, 2, limits).is_err());
    }
}

impl Floor1 {
    /// Decode encoded Y values. Nominal packet exhaustion yields an unused floor;
    /// setup/allocation errors remain errors. Curve prediction is a later stage.
    pub fn decode_y(
        &self,
        bits: &mut PacketBits<'_>,
        books: &[Codebook],
    ) -> Result<Option<Vec<u32>>> {
        let result = self.decode_y_inner(bits, books);
        match result {
            Err(_) if bits.ended => Ok(None),
            other => other,
        }
    }
    fn decode_y_inner(
        &self,
        bits: &mut PacketBits<'_>,
        books: &[Codebook],
    ) -> Result<Option<Vec<u32>>> {
        let range = *[256u32, 128, 86, 64]
            .get(usize::from(self.multiplier.wrapping_sub(1)))
            .ok_or_else(|| invalid("invalid Vorbis floor multiplier"))?;
        let mut values = 2usize;
        for &index in &self.partitions {
            let class = self
                .classes
                .get(usize::from(index))
                .ok_or_else(|| invalid("invalid Vorbis floor partition class"))?;
            if !(1..=8).contains(&class.dimensions)
                || class.subclasses > 3
                || class.books.len() != (1usize << class.subclasses)
            {
                return Err(invalid("invalid Vorbis floor class geometry"));
            }
            if class.subclasses > 0 && class.masterbook.is_none() {
                return Err(invalid("missing Vorbis floor masterbook"));
            }
            for index in class
                .books
                .iter()
                .flatten()
                .copied()
                .chain(class.masterbook)
            {
                if usize::from(index) >= books.len() {
                    return Err(invalid("invalid Vorbis floor packet book reference"));
                }
            }
            values += usize::from(class.dimensions);
        }
        if values > 65 || self.x.len() != values {
            return Err(invalid("invalid Vorbis floor point geometry"));
        }
        if bits.read(1)? == 0 {
            return Ok(None);
        }
        let width = (u32::BITS - (range - 1).leading_zeros()) as u8;
        let mut y = reserve(values)?;
        y.push(bits.read(width)?);
        y.push(bits.read(width)?);
        for &index in &self.partitions {
            let class = &self.classes[usize::from(index)];
            let mask = (1usize << class.subclasses) - 1;
            let mut selection = if class.subclasses > 0 {
                books[usize::from(class.masterbook.unwrap())].scalar(bits)?
            } else {
                0
            };
            for _ in 0..class.dimensions {
                let book = class.books[selection & mask];
                selection >>= class.subclasses;
                y.push(match book {
                    Some(book) => books[usize::from(book)].scalar(bits)? as u32,
                    None => 0,
                });
            }
        }
        Ok(Some(y))
    }
}
#[cfg(test)]
mod floor_packet_tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        for &(v, n) in fields {
            for i in 0..n {
                if at % 8 == 0 {
                    out.push(0);
                }
                out[at / 8] |= (((v >> i) & 1) as u8) << (at % 8);
                at += 1;
            }
        }
        out
    }
    fn book() -> Codebook {
        let bytes = pack(&[
            (0x564342, 24),
            (1, 16),
            (4, 24),
            (1, 1),
            (1, 5),
            (4, 3),
            (0, 4),
        ]);
        Codebook::parse(&mut PacketBits::new(&bytes), 4, 7, 0).unwrap()
    }
    #[test]
    fn floor_y_reads_subclass_choices_and_nominal_truncation() {
        let books = vec![book(), book()];
        let config = Floor1 {
            partitions: vec![0],
            classes: vec![FloorClass {
                dimensions: 2,
                subclasses: 1,
                masterbook: Some(0),
                books: vec![None, Some(1)],
            }],
            multiplier: 1,
            x: vec![0, 128, 64, 32],
        };
        // Master symbol 1 chooses book 1 then no book. Huffman symbols use MSB wire order.
        let bytes = pack(&[(1, 1), (120, 8), (30, 8), (2, 2), (3, 2)]);
        let mut bits = PacketBits::new(&bytes);
        assert_eq!(
            config.decode_y(&mut bits, &books).unwrap(),
            Some(vec![120, 30, 3, 0])
        );
        assert_eq!(bits.position(), 21);
        for end in 0..bytes.len() {
            assert!(
                config
                    .decode_y(&mut PacketBits::new(&bytes[..end]), &books)
                    .unwrap()
                    .is_none()
            );
        }
        let mut bits = PacketBits::new(&[0]);
        assert!(config.decode_y(&mut bits, &books).unwrap().is_none());
        assert_eq!(bits.position(), 1);
        assert!(config.decode_y(&mut PacketBits::new(&bytes), &[]).is_err());
    }
    #[test]
    fn floor_y_supports_all_multipliers_and_no_subclass_book() {
        for (multiplier, width) in [(1, 8), (2, 7), (3, 7), (4, 6)] {
            let config = Floor1 {
                partitions: vec![0],
                classes: vec![FloorClass {
                    dimensions: 1,
                    subclasses: 0,
                    masterbook: None,
                    books: vec![None],
                }],
                multiplier,
                x: vec![0, 128, 64],
            };
            let bytes = pack(&[(1, 1), (17, width), (42, width)]);
            let mut bits = PacketBits::new(&bytes);
            assert_eq!(
                config.decode_y(&mut bits, &[]).unwrap(),
                Some(vec![17, 42, 0])
            );
            assert_eq!(bits.position(), 1 + 2 * usize::from(width));
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct FloorPoints {
    pub y: Vec<u16>,
    pub active: Vec<bool>,
}
impl Floor1 {
    /// Reconstruct floor amplitudes in setup-list order with bounded workspace.
    pub fn predict(&self, encoded: &[u32]) -> Result<FloorPoints> {
        let range = i64::from(
            *[256u16, 128, 86, 64]
                .get(usize::from(self.multiplier.wrapping_sub(1)))
                .ok_or_else(|| invalid("invalid Vorbis floor multiplier"))?,
        );
        if !(2..=65).contains(&self.x.len())
            || encoded.len() != self.x.len()
            || self.x[0] != 0
            || self.x[1] == 0
        {
            return Err(invalid("invalid Vorbis floor prediction geometry"));
        }
        let mut y = reserve(encoded.len())?;
        let mut active = reserve(encoded.len())?;
        y.extend(
            encoded
                .iter()
                .map(|&v| i64::from(v).clamp(0, range - 1) as u16),
        );
        active.resize(encoded.len(), false);
        active[0] = true;
        active[1] = true;
        for i in 2..self.x.len() {
            let x = self.x[i];
            if self.x[..i].contains(&x) {
                return Err(invalid("duplicate Vorbis floor prediction X"));
            }
            let low = (0..i)
                .filter(|&j| self.x[j] < x)
                .max_by_key(|&j| self.x[j])
                .ok_or_else(|| invalid("missing Vorbis floor lower neighbor"))?;
            let high = (0..i)
                .filter(|&j| self.x[j] > x)
                .min_by_key(|&j| self.x[j])
                .ok_or_else(|| invalid("missing Vorbis floor upper neighbor"))?;
            let dy = i64::from(y[high]) - i64::from(y[low]);
            let offset =
                dy.abs() * i64::from(x - self.x[low]) / i64::from(self.x[high] - self.x[low]);
            let predicted = i64::from(y[low]) + if dy < 0 { -offset } else { offset };
            let lowroom = predicted;
            let highroom = range - predicted;
            let room = 2 * lowroom.min(highroom);
            let value = i64::from(encoded[i]);
            let final_y = if value == 0 {
                predicted
            } else {
                active[low] = true;
                active[high] = true;
                active[i] = true;
                if value >= room {
                    if highroom > lowroom {
                        value - lowroom + predicted
                    } else {
                        predicted - value + highroom - 1
                    }
                } else if value & 1 != 0 {
                    predicted - (value + 1) / 2
                } else {
                    predicted + value / 2
                }
            };
            y[i] = final_y.clamp(0, range - 1) as u16;
        }
        Ok(FloorPoints { y, active })
    }
    /// Render integer dB lookup indices into caller-owned spectral storage.
    pub fn curve_indices(&self, encoded: &[u32], output: &mut [u8]) -> Result<()> {
        let points = self.predict(encoded)?;
        let mut order = [0usize; 65];
        for (i, index) in order[..self.x.len()].iter_mut().enumerate() {
            *index = i;
        }
        order[..self.x.len()].sort_unstable_by_key(|&i| self.x[i]);
        let mut left = order[0];
        for &right in &order[1..self.x.len()] {
            if !points.active[right] {
                continue;
            }
            let x0 = usize::from(self.x[left]);
            let x1 = usize::from(self.x[right]);
            let y0 = i64::from(points.y[left]) * i64::from(self.multiplier);
            let y1 = i64::from(points.y[right]) * i64::from(self.multiplier);
            let dy = y1 - y0;
            for (x, value) in output.iter_mut().enumerate().take(x1).skip(x0) {
                let offset = dy.abs() * (x - x0) as i64 / (x1 - x0) as i64;
                *value = (y0 + if dy < 0 { -offset } else { offset }) as u8;
            }
            left = right;
        }
        let start = usize::from(self.x[left]).min(output.len());
        output[start..].fill((points.y[left] * u16::from(self.multiplier)) as u8);
        Ok(())
    }
}
#[cfg(test)]
mod floor_prediction_tests {
    use super::*;
    fn floor(x: Vec<u16>) -> Floor1 {
        Floor1 {
            partitions: vec![],
            classes: vec![],
            multiplier: 1,
            x,
        }
    }
    #[test]
    fn floor_prediction_unwraps_both_rooms_odd_even_and_zero_values() {
        let config = floor(vec![0, 128, 64]);
        for (a, b, val, expected) in [
            (110, 20, 9, 60),
            (10, 10, 30, 30),
            (240, 240, 40, 215),
            (60, 60, 8, 64),
            (60, 60, 7, 56),
            (60, 60, 0, 60),
        ] {
            let points = config.predict(&[a, b, val]).unwrap();
            assert_eq!(points.y, vec![a as u16, b as u16, expected]);
            assert_eq!(points.active, vec![true, true, val != 0]);
        }
        let config = floor(vec![0, 128, 64, 32]);
        let points = config.predict(&[110, 20, 0, 1]).unwrap();
        assert_eq!(points.y, vec![110, 20, 65, 87]);
        assert_eq!(points.active, vec![true, true, true, true]);
        assert!(floor(vec![0, 128, 128]).predict(&[1, 1, 1]).is_err());
    }
    #[test]
    fn floor_curve_uses_integer_rounding_sorted_active_points_and_constant_tail() {
        let config = floor(vec![0, 8, 4]);
        let mut curve = [99; 12];
        config.curve_indices(&[20, 4, 0], &mut curve).unwrap();
        assert_eq!(curve, [20, 18, 16, 14, 12, 10, 8, 6, 4, 4, 4, 4]);
        config.curve_indices(&[4, 20, 0], &mut curve[..7]).unwrap();
        assert_eq!(&curve[..7], &[4, 6, 8, 10, 12, 14, 16]);
        let config = floor(vec![0, 8]);
        config.curve_indices(&[10, 7], &mut curve).unwrap();
        assert_eq!(curve, [10, 10, 10, 9, 9, 9, 8, 8, 7, 7, 7, 7]);
    }
}

#[derive(Debug, PartialEq)]
pub struct FloorLsp {
    pub amplitude: u64,
    pub coefficients: Vec<f32>,
}
impl Floor0 {
    /// Decode LSP coefficients, retaining exactly `order` scalars while consuming
    /// the final complete codebook vector. Workspace admission is in f32 values.
    pub fn decode_lsp(
        &self,
        bits: &mut PacketBits<'_>,
        books: &[Codebook],
        max_values: usize,
    ) -> Result<Option<FloorLsp>> {
        let result = self.decode_lsp_inner(bits, books, max_values);
        match result {
            Err(_) if bits.ended => Ok(None),
            other => other,
        }
    }
    fn decode_lsp_inner(
        &self,
        bits: &mut PacketBits<'_>,
        books: &[Codebook],
        max_values: usize,
    ) -> Result<Option<FloorLsp>> {
        if self.order == 0
            || self.rate == 0
            || self.bark_map_size == 0
            || self.amplitude_bits > 63
            || self.books.is_empty()
            || self.books.len() > 16
        {
            return Err(invalid("invalid Vorbis floor zero geometry"));
        }
        let mut largest = 0;
        for &index in &self.books {
            let book = books
                .get(usize::from(index))
                .ok_or_else(|| invalid("invalid Vorbis floor zero book"))?;
            if book.lookup.is_none() || book.dimensions == 0 {
                return Err(invalid("Vorbis floor zero requires vector books"));
            }
            largest = largest.max(usize::from(book.dimensions));
        }
        if usize::from(self.order) + largest > max_values {
            return Err(invalid("Vorbis LSP workspace exceeds value budget"));
        }
        let low = bits.read(self.amplitude_bits.min(32))?;
        let high = if self.amplitude_bits > 32 {
            bits.read(self.amplitude_bits - 32)?
        } else {
            0
        };
        let amplitude = u64::from(low) | (u64::from(high) << 32);
        if amplitude == 0 {
            return Ok(None);
        }
        let width = (usize::BITS - self.books.len().leading_zeros()) as u8;
        let selected = bits.read(width)? as usize;
        let index = *self
            .books
            .get(selected)
            .ok_or_else(|| invalid("reserved Vorbis floor zero book selection"))?;
        let book = &books[usize::from(index)];
        let mut scratch = reserve(usize::from(book.dimensions))?;
        scratch.resize(usize::from(book.dimensions), 0.);
        let mut coefficients = reserve(usize::from(self.order))?;
        let mut last = 0.;
        while coefficients.len() < usize::from(self.order) {
            book.vector(bits, &mut scratch)?;
            for value in &mut scratch {
                *value += last;
            }
            last = *scratch.last().unwrap();
            let count = (usize::from(self.order) - coefficients.len()).min(scratch.len());
            coefficients.extend_from_slice(&scratch[..count]);
        }
        Ok(Some(FloorLsp {
            amplitude,
            coefficients,
        }))
    }
}
#[cfg(test)]
mod floor_lsp_tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        for &(v, n) in fields {
            for i in 0..n {
                if at % 8 == 0 {
                    out.push(0);
                }
                out[at / 8] |= (((v >> i) & 1) as u8) << (at % 8);
                at += 1;
            }
        }
        out
    }
    fn book() -> Codebook {
        let bytes = pack(&[
            (0x564342, 24),
            (3, 16),
            (1, 24),
            (0, 1),
            (0, 1),
            (0, 5),
            (2, 4),
            (0, 32),
            (788 << 21 | 1, 32),
            (1, 4),
            (0, 1),
            (1, 2),
            (2, 2),
            (3, 2),
        ]);
        Codebook::parse(&mut PacketBits::new(&bytes), 1, 1, 3).unwrap()
    }
    fn floor(bits: u8) -> Floor0 {
        Floor0 {
            order: 5,
            rate: 48000,
            bark_map_size: 64,
            amplitude_bits: bits,
            amplitude_offset: 120,
            books: vec![0],
        }
    }
    #[test]
    fn floor_zero_consumes_complete_vectors_accumulates_and_discards_extra_lsp() {
        let books = vec![book()];
        let bytes = pack(&[(17, 6), (0, 1), (0, 1), (1, 1)]);
        let mut bits = PacketBits::new(&bytes);
        assert_eq!(
            floor(6).decode_lsp(&mut bits, &books, 8).unwrap(),
            Some(FloorLsp {
                amplitude: 17,
                coefficients: vec![1., 2., 3., 4., 5.]
            })
        );
        assert_eq!(bits.position(), 9);
        for end in 0..bytes.len() {
            assert!(
                floor(6)
                    .decode_lsp(&mut PacketBits::new(&bytes[..end]), &books, 8)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            floor(6)
                .decode_lsp(&mut PacketBits::new(&bytes), &books, 7)
                .is_err()
        );
        let bytes = pack(&[(1, 6), (1, 1)]);
        assert!(
            floor(6)
                .decode_lsp(&mut PacketBits::new(&bytes), &books, 8)
                .is_err()
        );
    }
    #[test]
    fn floor_zero_supports_wide_amplitude_fields_without_truncation() {
        let books = vec![book()];
        let bytes = pack(&[(0xabcdef01, 32), (0x456789ab, 31), (0, 1), (0, 1), (0, 1)]);
        let mut bits = PacketBits::new(&bytes);
        let decoded = floor(63).decode_lsp(&mut bits, &books, 8).unwrap().unwrap();
        assert_eq!(decoded.amplitude, 0x456789ababcdef01);
        assert_eq!(bits.position(), 66);
        let bytes = pack(&[(0, 6)]);
        assert!(
            floor(6)
                .decode_lsp(&mut PacketBits::new(&bytes), &books, 8)
                .unwrap()
                .is_none()
        );
    }
}

fn bark(frequency: f64) -> f64 {
    13.1 * (0.00074 * frequency).atan()
        + 2.24 * (0.0000000185 * frequency * frequency).atan()
        + 0.0001 * frequency
}
impl Floor0 {
    /// Synthesize the floor envelope in caller-owned spectral storage.
    pub fn curve(&self, lsp: &FloorLsp, output: &mut [f32]) -> Result<()> {
        if self.order == 0
            || self.rate == 0
            || self.bark_map_size == 0
            || self.amplitude_bits > 63
            || lsp.coefficients.len() != usize::from(self.order)
        {
            return Err(invalid("invalid Vorbis LSP curve geometry"));
        }
        let maximum = (1u64 << self.amplitude_bits) - 1;
        if lsp.amplitude > maximum {
            return Err(invalid("Vorbis floor amplitude exceeds encoded width"));
        }
        if lsp.amplitude == 0 {
            output.fill(0.);
            return Ok(());
        }
        let mut cosines = [0f64; 255];
        for (to, &coefficient) in cosines.iter_mut().zip(&lsp.coefficients) {
            if !coefficient.is_finite() {
                return Err(invalid("nonfinite Vorbis LSP coefficient"));
            }
            *to = f64::from(coefficient).cos();
        }
        let scale = f64::from(self.bark_map_size) / bark(f64::from(self.rate) * 0.5);
        let amplitude = lsp.amplitude as f64 * f64::from(self.amplitude_offset) / maximum as f64;
        let bins = output.len();
        let mut previous = None;
        let mut value = 0.;
        for (i, sample) in output.iter_mut().enumerate() {
            let map = (bark(f64::from(self.rate) * i as f64 / (2. * bins as f64)) * scale)
                .floor()
                .min(f64::from(self.bark_map_size - 1)) as u16;
            if previous != Some(map) {
                let w =
                    (std::f64::consts::PI * f64::from(map) / f64::from(self.bark_map_size)).cos();
                let odd = self.order & 1 != 0;
                let (mut p, mut q) = if odd {
                    (1. - w * w, 0.25)
                } else {
                    ((1. - w) * 0.5, (1. + w) * 0.5)
                };
                for (j, &coefficient) in cosines[..usize::from(self.order)].iter().enumerate() {
                    let factor = 4. * (coefficient - w).powi(2);
                    if j & 1 == 0 {
                        q *= factor;
                    } else {
                        p *= factor;
                    }
                }
                value = (0.11512925
                    * (amplitude / (p + q).sqrt() - f64::from(self.amplitude_offset)))
                .exp() as f32;
                previous = Some(map);
            }
            *sample = value;
        }
        Ok(())
    }
}
#[cfg(test)]
mod floor_curve_tests {
    use super::*;
    fn config(order: u8, map: u16) -> Floor0 {
        Floor0 {
            order,
            rate: 48000,
            bark_map_size: map,
            amplitude_bits: 6,
            amplitude_offset: 120,
            books: vec![0],
        }
    }
    #[test]
    fn one_lsp_at_pi_over_two_has_a_constant_analytic_envelope() {
        for bins in [1, 32, 128, 1024] {
            for map in [1, 8, 64, 65535] {
                let mut curve = vec![0.; bins];
                let lsp = FloorLsp {
                    amplitude: 17,
                    coefficients: vec![std::f32::consts::FRAC_PI_2],
                };
                config(1, map).curve(&lsp, &mut curve).unwrap();
                // p=1-w², q=w², hence sqrt(p+q)=1 independently of Bark bin.
                let expected = (0.11512925f64 * (17. * 120. / 63. - 120.)).exp() as f32;
                for value in curve {
                    assert!((value - expected).abs() < expected * 0.00001);
                }
            }
        }
    }
    #[test]
    fn even_order_bark_zero_bin_matches_closed_form_and_invalid_input_is_atomic() {
        let floor = config(2, 1);
        let lsp = FloorLsp {
            amplitude: 17,
            coefficients: vec![std::f32::consts::FRAC_PI_2, 0.25],
        };
        let mut curve = [0.; 64];
        floor.curve(&lsp, &mut curve).unwrap();
        // map=0, w=1; p=0, q=4*(cos(pi/2)-1)^2, so sqrt=2.
        let expected = (0.11512925f64 * (17. * 120. / 63. / 2. - 120.)).exp() as f32;
        for value in curve {
            assert!((value - expected).abs() < expected * 0.00001);
        }
        let invalid = FloorLsp {
            amplitude: 17,
            coefficients: vec![f32::NAN, 0.25],
        };
        let mut output = [42.; 8];
        assert!(floor.curve(&invalid, &mut output).is_err());
        assert_eq!(output, [42.; 8]);
        floor
            .curve(
                &FloorLsp {
                    amplitude: 0,
                    coefficients: vec![0., 0.],
                },
                &mut output,
            )
            .unwrap();
        assert_eq!(output, [0.; 8]);
    }
}

#[path = "vorbis_inverse_db.rs"]
mod inverse_db;
impl Floor1 {
    /// Convert the integer floor curve through the normative inverse-dB table.
    /// Caller supplies both spectral output and equal-length byte scratch.
    pub fn curve(&self, encoded: &[u32], output: &mut [f32], scratch: &mut [u8]) -> Result<()> {
        if output.len() != scratch.len() {
            return Err(invalid("Vorbis floor curve scratch length mismatch"));
        }
        self.curve_indices(encoded, scratch)?;
        for (sample, &index) in output.iter_mut().zip(scratch.iter()) {
            *sample = inverse_db::INVERSE_DB[usize::from(index)];
        }
        Ok(())
    }
}
#[cfg(test)]
mod inverse_db_tests {
    use super::*;
    #[test]
    fn every_floor_db_index_uses_the_normative_table_and_monotonic_scale() {
        let floor = Floor1 {
            partitions: vec![],
            classes: vec![],
            multiplier: 1,
            x: vec![0, 8],
        };
        let mut previous = 0.;
        for index in 0..256 {
            let mut output = [0.; 8];
            let mut scratch = [0; 8];
            floor
                .curve(&[index, index], &mut output, &mut scratch)
                .unwrap();
            assert_eq!(output, [inverse_db::INVERSE_DB[index as usize]; 8]);
            assert!(output[0] > previous);
            previous = output[0];
            // Independently check the ~140 dB span and uniform logarithmic step.
            let expected = ((f64::from(index) - 255.) * 140. / 256. * std::f64::consts::LN_10 / 20.)
                .exp() as f32;
            assert!((output[0] - expected).abs() < expected * 0.00001);
        }
        assert_eq!(previous, 1.);
        let mut output = [42.; 8];
        assert!(floor.curve(&[1, 1], &mut output, &mut [0; 7]).is_err());
        assert_eq!(output, [42.; 8]);
    }
}

impl Residue {
    /// Decode the submap bundle. Packet exhaustion preserves completed vector
    /// additions; malformed configuration and workspace admission return errors.
    pub fn decode(
        &self,
        bits: &mut PacketBits<'_>,
        books: &[Codebook],
        nonzero: &[bool],
        output: &mut [&mut [f32]],
        max_values: usize,
    ) -> Result<()> {
        if self.kind > 2
            || self.partition_size == 0
            || self.books.is_empty()
            || output.len() != nonzero.len()
        {
            return Err(invalid("invalid Vorbis residue packet geometry"));
        }
        let size = output.first().map_or(0, |v| v.len());
        if output.iter().any(|v| v.len() != size) {
            return Err(invalid("unequal Vorbis residue vectors"));
        }
        let class = books
            .get(usize::from(self.classbook))
            .ok_or_else(|| invalid("invalid Vorbis residue classbook"))?;
        if class.dimensions == 0 {
            return Err(invalid("zero Vorbis classbook dimension"));
        }
        for index in self.books.iter().flatten().flatten() {
            let book = books
                .get(usize::from(*index))
                .ok_or_else(|| invalid("invalid Vorbis residue vector book"))?;
            if book.lookup.is_none()
                || book.dimensions == 0
                || self.partition_size % u32::from(book.dimensions) != 0
            {
                return Err(invalid("invalid Vorbis residue partition book geometry"));
            }
        }
        for vector in output.iter_mut() {
            vector.fill(0.);
        }
        if output.is_empty() || !nonzero.iter().any(|&used| used) {
            return Ok(());
        }
        let result = if self.kind == 2 {
            let values = size
                .checked_mul(output.len())
                .ok_or_else(|| invalid("Vorbis residue interleave overflow"))?;
            if values > max_values {
                return Err(invalid("Vorbis residue workspace exceeds value budget"));
            }
            let mut flat = reserve(values)?;
            flat.resize(values, 0.);
            let result = self.decode_vectors(
                bits,
                books,
                &[true],
                &mut [flat.as_mut_slice()],
                1,
                max_values - values,
            );
            let channels = output.len();
            for (channel, vector) in output.iter_mut().enumerate() {
                for (i, sample) in vector.iter_mut().enumerate() {
                    *sample = flat[i * channels + channel];
                }
            }
            result
        } else {
            self.decode_vectors(bits, books, nonzero, output, self.kind, max_values)
        };
        match result {
            Err(_) if bits.ended => Ok(()),
            other => other,
        }
    }
    fn decode_vectors(
        &self,
        bits: &mut PacketBits<'_>,
        books: &[Codebook],
        nonzero: &[bool],
        output: &mut [&mut [f32]],
        kind: u16,
        max_values: usize,
    ) -> Result<()> {
        let size = output.first().map_or(0, |v| v.len());
        let begin = (self.begin as usize).min(size);
        let end = (self.end as usize).min(size);
        let partition = self.partition_size as usize;
        let count = end.saturating_sub(begin) / partition;
        if count == 0 {
            return Ok(());
        }
        let class = &books[usize::from(self.classbook)];
        let words = usize::from(class.dimensions);
        let per_channel = count
            .div_ceil(words)
            .checked_mul(words)
            .ok_or_else(|| invalid("Vorbis classification size overflow"))?;
        let classification_count = per_channel
            .checked_mul(output.len())
            .ok_or_else(|| invalid("Vorbis classification workspace overflow"))?;
        let max_dimension = self
            .books
            .iter()
            .flatten()
            .flatten()
            .map(|&i| usize::from(books[usize::from(i)].dimensions))
            .max()
            .unwrap_or(0);
        if classification_count
            .checked_add(max_dimension)
            .is_none_or(|n| n > max_values)
        {
            return Err(invalid("Vorbis residue workspace exceeds value budget"));
        }
        let mut classifications = reserve(classification_count)?;
        classifications.resize(classification_count, 0usize);
        let mut vector = reserve(max_dimension)?;
        vector.resize(max_dimension, 0.);
        for pass in 0..8 {
            let mut at = 0;
            while at < count {
                if pass == 0 {
                    for (channel, &used) in nonzero.iter().enumerate() {
                        if used {
                            let mut value = class.scalar(bits)?;
                            for digit in (0..words).rev() {
                                classifications[channel * per_channel + at + digit] =
                                    value % self.books.len();
                                value /= self.books.len();
                            }
                        }
                    }
                }
                for _ in 0..words {
                    if at >= count {
                        break;
                    }
                    for (channel, &used) in nonzero.iter().enumerate() {
                        if used {
                            let classification = classifications[channel * per_channel + at];
                            if let Some(index) = self.books[classification][pass] {
                                let book = &books[usize::from(index)];
                                let dimension = usize::from(book.dimensions);
                                let destination = &mut output[channel]
                                    [begin + at * partition..begin + (at + 1) * partition];
                                let step = partition / dimension;
                                for i in 0..step {
                                    book.vector(bits, &mut vector[..dimension])?;
                                    for (j, &value) in vector[..dimension].iter().enumerate() {
                                        let offset = if kind == 0 {
                                            i + j * step
                                        } else {
                                            i * dimension + j
                                        };
                                        destination[offset] += value;
                                    }
                                }
                            }
                        }
                    }
                    at += 1;
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod residue_packet_tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        for &(v, n) in fields {
            for i in 0..n {
                if at % 8 == 0 {
                    out.push(0);
                }
                out[at / 8] |= (((v >> i) & 1) as u8) << (at % 8);
                at += 1;
            }
        }
        out
    }
    fn books() -> Vec<Codebook> {
        let scalar = pack(&[
            (0x564342, 24),
            (1, 16),
            (1, 24),
            (0, 1),
            (0, 1),
            (0, 5),
            (0, 4),
        ]);
        let vector = pack(&[
            (0x564342, 24),
            (2, 16),
            (2, 24),
            (1, 1),
            (0, 5),
            (2, 2),
            (2, 4),
            (0, 32),
            (788 << 21 | 1, 32),
            (1, 4),
            (0, 1),
            (1, 2),
            (2, 2),
            (3, 2),
            (0, 2),
        ]);
        [scalar, vector]
            .iter()
            .map(|bytes| Codebook::parse(&mut PacketBits::new(bytes), 2, 3, 4).unwrap())
            .collect()
    }
    fn residue(kind: u16) -> Residue {
        Residue {
            kind,
            begin: 1,
            end: 5,
            partition_size: 4,
            classbook: 0,
            books: vec![[Some(1), None, None, None, None, None, None, None]],
        }
    }
    #[test]
    fn residue_zero_and_one_place_vectors_differently_and_skip_unused_channels() {
        let books = books();
        let bytes = pack(&[(0, 1), (0, 1), (1, 1)]);
        for (kind, expected) in [
            (0, vec![0., 1., 3., 2., 0., 0.]),
            (1, vec![0., 1., 2., 3., 0., 0.]),
        ] {
            let (mut first, mut skipped) = (vec![99.; 6], vec![99.; 6]);
            let mut bits = PacketBits::new(&bytes);
            residue(kind)
                .decode(
                    &mut bits,
                    &books,
                    &[true, false],
                    &mut [&mut first, &mut skipped],
                    32,
                )
                .unwrap();
            assert_eq!(first, expected);
            assert_eq!(skipped, vec![0.; 6]);
            assert_eq!(bits.position(), 3);
        }
    }
    #[test]
    fn residue_two_interleaves_all_channels_if_any_is_used() {
        let books = books();
        let bytes = pack(&[(0, 1), (0, 1), (1, 1)]);
        let mut config = residue(2);
        config.begin = 0;
        config.end = 4;
        let (mut a, mut b) = (vec![99.; 3], vec![99.; 3]);
        let mut bits = PacketBits::new(&bytes);
        config
            .decode(&mut bits, &books, &[false, true], &mut [&mut a, &mut b], 32)
            .unwrap();
        assert_eq!(a, vec![1., 3., 0.]);
        assert_eq!(b, vec![2., 0., 0.]);
        let mut bits = PacketBits::new(&[]);
        config
            .decode(&mut bits, &books, &[false, false], &mut [&mut a, &mut b], 0)
            .unwrap();
        assert_eq!(a, vec![0.; 3]);
        assert_eq!(b, vec![0.; 3]);
        assert_eq!(bits.position(), 0);
        assert!(
            config
                .decode(
                    &mut PacketBits::new(&bytes),
                    &books,
                    &[true, true],
                    &mut [&mut a, &mut b],
                    1
                )
                .is_err()
        );
    }
    #[test]
    fn residue_passes_accumulate_and_packet_exhaustion_retains_completed_vectors() {
        let books = books();
        let mut config = residue(1);
        config.begin = 0;
        config.end = 4;
        config.books[0][7] = Some(1);
        let bytes = pack(&[(0, 1), (0, 1), (1, 1), (1, 1), (0, 1)]);
        let mut output = [0.; 4];
        let mut bits = PacketBits::new(&bytes);
        config
            .decode(&mut bits, &books, &[true], &mut [&mut output], 32)
            .unwrap();
        assert_eq!(output, [4., 2., 4., 2.]);
        assert_eq!(bits.position(), 5);
        config.end = 16;
        let mut output = [99.; 16];
        let mut bits = PacketBits::new(&[0]);
        config
            .decode(&mut bits, &books, &[true], &mut [&mut output], 64)
            .unwrap();
        assert!(bits.ended);
        assert_eq!(&output[..8], &[1., 2., 1., 2., 1., 2., 1., 2.]);
        assert_eq!(&output[8..], &[1., 2., 0., 0., 0., 0., 0., 0.]);
    }
}

pub(crate) fn huffman_node_bytes() -> usize {
    std::mem::size_of::<HuffmanNode>()
}
