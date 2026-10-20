//! Owned bounded Layer III Huffman, escape and sign decoding.
use crate::{Result, invalid};
use std::{ops::Range, sync::OnceLock};
#[path = "mp3_codebooks.rs"]
mod codebooks;
#[derive(Clone, Copy)]
struct Node {
    children: [u16; 2],
    symbol: i16,
}
struct Tree {
    nodes: Vec<Node>,
}
fn build() -> std::result::Result<Vec<Tree>, &'static str> {
    let mut trees = Vec::new();
    trees
        .try_reserve_exact(17)
        .map_err(|_| "MP3 Huffman tree allocation failed")?;
    for book in codebooks::BOOKS {
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(book.len() * 2)
            .map_err(|_| "MP3 Huffman node allocation failed")?;
        nodes.push(Node {
            children: [u16::MAX; 2],
            symbol: -1,
        });
        for &(word, length, symbol) in book {
            if length == 0 || length > 19 || word >= (1 << length) {
                return Err("invalid MP3 codeword data");
            }
            let mut at = 0;
            for bit in (0..length).rev() {
                if nodes[at].symbol >= 0 {
                    return Err("MP3 Huffman code has a leaf prefix");
                }
                let side = ((word >> bit) & 1) as usize;
                let mut next = nodes[at].children[side];
                if next == u16::MAX {
                    next = u16::try_from(nodes.len())
                        .map_err(|_| "MP3 Huffman tree index overflow")?;
                    nodes.push(Node {
                        children: [u16::MAX; 2],
                        symbol: -1,
                    });
                    nodes[at].children[side] = next;
                }
                at = usize::from(next);
            }
            if nodes[at].symbol >= 0 || nodes[at].children != [u16::MAX; 2] {
                return Err("MP3 Huffman codewords are not prefix free");
            }
            nodes[at].symbol = i16::from(symbol);
        }
        if nodes
            .iter()
            .any(|node| node.symbol < 0 && node.children.contains(&u16::MAX))
        {
            return Err("incomplete MP3 Huffman codebook");
        }
        trees.push(Tree { nodes });
    }
    Ok(trees)
}
/// Conservative allocation admission for the lazily constructed numerical trees.
pub(crate) fn admission_bytes() -> usize {
    17 * std::mem::size_of::<Tree>()
        + codebooks::BOOKS.iter().map(|book| book.len() * 2 * std::mem::size_of::<Node>()).sum::<usize>()
}
fn trees() -> Result<&'static [Tree]> {
    static TREES: OnceLock<std::result::Result<Vec<Tree>, &'static str>> = OnceLock::new();
    TREES
        .get_or_init(build)
        .as_ref()
        .map(|v| v.as_slice())
        .map_err(|e| invalid(e))
}
pub struct Spectrum {
    pub values: [i32; 576],
    pub nonzero_end: usize,
    pub huffman_bits: usize,
}

#[derive(Clone)]
pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}
impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8], range: Range<usize>) -> Result<Self> {
        let bits = data
            .len()
            .checked_mul(8)
            .ok_or_else(|| invalid("MP3 bit buffer length overflow"))?;
        if range.start > range.end || range.end > bits {
            return Err(invalid("MP3 granule bit range outside buffer"));
        }
        Ok(Self {
            data,
            pos: range.start,
            end: range.end,
        })
    }
    pub fn position(&self) -> usize {
        self.pos
    }
    pub fn remaining(&self) -> usize {
        self.end - self.pos
    }
    pub fn read(&mut self, n: usize) -> Result<u32> {
        if n > 32 || n > self.remaining() {
            return Err(invalid("MP3 Huffman read exceeds granule boundary"));
        }
        let mut word = 0;
        for _ in 0..n {
            word = (word << 1) | u32::from((self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Ok(word)
    }
    fn symbol(&mut self, tree: &Tree) -> Result<u8> {
        let mut node = 0;
        for _ in 0..=19 {
            let item = &tree.nodes[node];
            if item.symbol >= 0 {
                return Ok(item.symbol as u8);
            }
            let side = self.read(1)? as usize;
            let next = item.children[side];
            if next == u16::MAX {
                return Err(invalid("invalid MP3 Huffman prefix"));
            }
            node = usize::from(next);
        }
        Err(invalid("MP3 Huffman codeword exceeds maximum length"))
    }
    /// Return one signed pair; failure leaves the cursor unchanged.
    pub fn pair(&mut self, table: u8) -> Result<[i32; 2]> {
        if table == 0 {
            return Ok([0; 2]);
        }
        let (index, linbits) = match table {
            1..=3 => (usize::from(table - 1), 0),
            5..=13 => (usize::from(table - 2), 0),
            15 => (12, 0),
            16..=23 => (13, [1, 2, 3, 4, 6, 8, 10, 13][usize::from(table - 16)]),
            24..=31 => (14, [4, 5, 6, 7, 8, 9, 11, 13][usize::from(table - 24)]),
            _ => return Err(invalid("reserved or invalid MP3 Huffman pair table")),
        };
        let mut next = self.clone();
        let symbol = next.symbol(&trees()?[index])?;
        let mut values = [i32::from(symbol >> 4), i32::from(symbol & 15)];
        for v in &mut values {
            if linbits > 0 && *v == 15 {
                *v += next.read(linbits)? as i32;
            }
            if *v != 0 && next.read(1)? != 0 {
                *v = -*v;
            }
        }
        *self = next;
        Ok(values)
    }
    /// Return one count1 quadruple; incomplete trailing words consume no bits.
    pub fn quad(&mut self, table: u8) -> Result<[i32; 4]> {
        if table > 1 {
            return Err(invalid("invalid MP3 count1 table"));
        }
        let mut next = self.clone();
        let symbol = next.symbol(&trees()?[15 + usize::from(table)])?;
        let mut values = std::array::from_fn(|i| i32::from((symbol >> (3 - i)) & 1));
        for v in &mut values {
            if *v != 0 && next.read(1)? != 0 {
                *v = -*v;
            }
        }
        *self = next;
        Ok(values)
    }
    /// Decode big_values regions and count1 into 576 quantized lines. The
    /// cursor begins after scalefactors and ends at the declared granule limit.
    /// An incomplete count1 suffix is stuffing; a failed big_values pair is an
    /// error. No failing granule advances the caller's cursor.
    pub fn spectrum(
        &mut self,
        big_values: u16,
        region_ends: [usize; 2],
        tables: [u8; 3],
        count1_table: u8,
    ) -> Result<Spectrum> {
        if big_values > 288 {
            return Err(invalid("MP3 big_values exceeds 288 pairs"));
        }
        if region_ends[0] > region_ends[1]
            || region_ends[1] > 576
            || region_ends.iter().any(|n| n % 2 != 0)
        {
            return Err(invalid("invalid MP3 Huffman region boundaries"));
        }
        if tables.iter().any(|&t| t > 31 || t == 4 || t == 14) || count1_table > 1 {
            return Err(invalid("invalid MP3 spectrum Huffman table selection"));
        }
        trees()?; // Complete static books: quad failure below can only be a boundary.
        let mut next = self.clone();
        let begin = next.position();
        let mut values = [0; 576];
        let mut at = 0;
        while at < usize::from(big_values) * 2 {
            let region = if at < region_ends[0] {
                0
            } else if at < region_ends[1] {
                1
            } else {
                2
            };
            let pair = next.pair(tables[region])?;
            values[at..at + 2].copy_from_slice(&pair);
            at += 2;
        }
        while at + 4 <= 576 && next.remaining() != 0 {
            // Quad parsing is transactional, including its sign bits. Partial
            // final words must not consume stuffing or the following granule.
            let Ok(quad) = next.quad(count1_table) else {
                break;
            };
            values[at..at + 4].copy_from_slice(&quad);
            at += 4;
        }
        let huffman_bits = next.position() - begin;
        next.pos = next.end;
        let nonzero_end = values.iter().rposition(|&v| v != 0).map_or(0, |i| i + 1);
        *self = next;
        Ok(Spectrum {
            values,
            nonzero_end,
            huffman_bits,
        })
    }
}
