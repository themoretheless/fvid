//! Owned prefix decoding over normative SBR protocol codewords.
use super::aac_sbr_huffman_tables::*;
use super::{Result, bits::BitReader, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Book {
    EnvelopeTime15,
    EnvelopeFrequency15,
    BalanceTime15,
    BalanceFrequency15,
    EnvelopeTime30,
    EnvelopeFrequency30,
    BalanceTime30,
    BalanceFrequency30,
    NoiseTime,
    NoiseBalanceTime,
}
impl Book {
    fn words(self) -> &'static [(u8, u32)] {
        match self {
            Self::EnvelopeTime15 => BOOK_0,
            Self::EnvelopeFrequency15 => BOOK_1,
            Self::BalanceTime15 => BOOK_2,
            Self::BalanceFrequency15 => BOOK_3,
            Self::EnvelopeTime30 => BOOK_4,
            Self::EnvelopeFrequency30 => BOOK_5,
            Self::BalanceTime30 => BOOK_6,
            Self::BalanceFrequency30 => BOOK_7,
            Self::NoiseTime => BOOK_8,
            Self::NoiseBalanceTime => BOOK_9,
        }
    }
    /// Return signed delta; failed or truncated reads do not consume any bits.
    pub fn decode(self, bits: &mut BitReader<'_>, end: usize) -> Result<i16> {
        if end < bits.position() || end - bits.position() > bits.remaining() {
            return Err(invalid("invalid SBR Huffman payload boundary"));
        }
        let words = self.words();
        let mut trial = bits.clone();
        let mut prefix = 0u32;
        for width in 1..=20 {
            if trial.position() == end {
                return Err(invalid("truncated SBR Huffman codeword"));
            }
            prefix = (prefix << 1) | trial.read(1)?;
            if let Some(index) = words.iter().position(|&(n, c)| n == width && c == prefix) {
                *bits = trial;
                return Ok(index as i16 - (words.len() / 2) as i16);
            }
        }
        Err(invalid("invalid SBR Huffman codeword"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BOOKS: [Book; 10] = [
        Book::EnvelopeTime15,
        Book::EnvelopeFrequency15,
        Book::BalanceTime15,
        Book::BalanceFrequency15,
        Book::EnvelopeTime30,
        Book::EnvelopeFrequency30,
        Book::BalanceTime30,
        Book::BalanceFrequency30,
        Book::NoiseTime,
        Book::NoiseBalanceTime,
    ];
    #[test]
    fn every_normative_symbol_decodes_and_every_truncation_rolls_back() {
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-huffman-codewords.json"
        ))
        .unwrap();
        for (book, table) in BOOKS.into_iter().zip(oracle.as_array().unwrap()) {
            let offset = table["offset"].as_i64().unwrap();
            for row in table["rows"].as_array().unwrap() {
                let index = row[0].as_i64().unwrap();
                let width = row[1].as_u64().unwrap() as usize;
                let code = row[2].as_u64().unwrap();
                for start in 0..8 {
                    let mut bytes = vec![255; (start + width + 16).div_ceil(8)];
                    for i in 0..width {
                        if code & (1 << (width - i - 1)) == 0 {
                            bytes[(start + i) / 8] &= !(1 << (7 - (start + i) % 8));
                        }
                    }
                    let mut bits = BitReader::new(&bytes);
                    bits.skip(start).unwrap();
                    for end in start..start + width {
                        assert!(book.decode(&mut bits, end).is_err());
                        assert_eq!(bits.position(), start);
                    }
                    assert_eq!(
                        book.decode(&mut bits, start + width).unwrap(),
                        (index - offset) as i16
                    );
                    assert_eq!(bits.position(), start + width);
                    assert_eq!(bits.read(8).unwrap(), 255);
                }
            }
        }
    }
    #[test]
    fn codebooks_are_complete_and_prefix_free() {
        for book in BOOKS {
            let words = book.words();
            let max = words.iter().map(|x| x.0).max().unwrap();
            assert_eq!(
                words.iter().map(|x| 1u32 << (max - x.0)).sum::<u32>(),
                1 << max
            );
            for (i, &(n, c)) in words.iter().enumerate() {
                assert!(n > 0 && n <= 20 && c < 1 << n);
                for &(m, d) in &words[i + 1..] {
                    assert!(if n <= m {
                        c != d >> (m - n)
                    } else {
                        d != c >> (n - m)
                    });
                }
            }
        }
    }
}
