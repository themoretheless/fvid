//! FVid-owned bounded prefix decoding of PS IID/ICC/IPD/OPD deltas.
use super::{Result, aac_ps_huffman_tables::*, bits::BitReader, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Book {
    IidCoarseFrequency,
    IidCoarseTime,
    IidFineFrequency,
    IidFineTime,
    IccFrequency,
    IccTime,
    IpdFrequency,
    IpdTime,
    OpdFrequency,
    OpdTime,
}

impl Book {
    fn words(self) -> &'static [(i16, u8, u32)] {
        match self {
            Self::IidCoarseFrequency => BOOK_0,
            Self::IidCoarseTime => BOOK_1,
            Self::IidFineFrequency => BOOK_2,
            Self::IidFineTime => BOOK_3,
            Self::IccFrequency => BOOK_4,
            Self::IccTime => BOOK_5,
            Self::IpdFrequency => BOOK_6,
            Self::IpdTime => BOOK_7,
            Self::OpdFrequency => BOOK_8,
            Self::OpdTime => BOOK_9,
        }
    }

    /// Absolute enclosing bit boundary. Failure never consumes input bits.
    pub fn decode(self, bits: &mut BitReader<'_>, end: usize) -> Result<i16> {
        if end < bits.position() || end - bits.position() > bits.remaining() {
            return Err(invalid("invalid PS Huffman payload boundary"));
        }
        let mut trial = bits.clone();
        let mut prefix = 0u32;
        let words = self.words();
        for width in 1..=20 {
            if trial.position() == end {
                return Err(invalid("truncated PS Huffman codeword"));
            }
            prefix = prefix << 1 | trial.read(1)?;
            if let Some(&(symbol, _, _)) = words
                .iter()
                .find(|&&(_, n, code)| n == width && code == prefix)
            {
                *bits = trial;
                return Ok(symbol);
            }
        }
        Err(invalid("invalid PS Huffman codeword"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BOOKS: [Book; 10] = [
        Book::IidCoarseFrequency,
        Book::IidCoarseTime,
        Book::IidFineFrequency,
        Book::IidFineTime,
        Book::IccFrequency,
        Book::IccTime,
        Book::IpdFrequency,
        Book::IpdTime,
        Book::OpdFrequency,
        Book::OpdTime,
    ];

    #[test]
    fn all_normative_words_and_bit_truncations_are_transactional() {
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ps-huffman-codewords.json"
        ))
        .unwrap();
        for (book, table) in BOOKS.into_iter().zip(oracle["books"].as_array().unwrap()) {
            for row in table["rows"].as_array().unwrap() {
                let symbol = row[0].as_i64().unwrap() as i16;
                let word = row[1].as_str().unwrap();
                for start in 0..8 {
                    let mut bytes = vec![255; (start + word.len() + 8).div_ceil(8)];
                    for (offset, digit) in word.bytes().enumerate() {
                        if digit == b'0' {
                            bytes[(start + offset) / 8] &= !(1 << (7 - (start + offset) % 8));
                        }
                    }
                    let mut bits = BitReader::new(&bytes);
                    bits.skip(start).unwrap();
                    for end in start..start + word.len() {
                        assert!(book.decode(&mut bits, end).is_err());
                        assert_eq!(bits.position(), start);
                    }
                    assert_eq!(book.decode(&mut bits, start + word.len()).unwrap(), symbol);
                    assert_eq!(bits.position(), start + word.len());
                    assert_eq!(bits.read(8).unwrap(), 255);
                }
            }
        }
    }

    #[test]
    fn books_are_complete_and_prefix_free() {
        for book in BOOKS {
            let words = book.words();
            let max = words.iter().map(|x| x.1).max().unwrap();
            assert_eq!(
                words.iter().map(|x| 1u32 << (max - x.1)).sum::<u32>(),
                1 << max
            );
            for (i, &(_, width, code)) in words.iter().enumerate() {
                assert!(width > 0 && width <= 20 && code < 1 << width);
                for &(_, other_width, other_code) in &words[i + 1..] {
                    assert!(if width <= other_width {
                        code != other_code >> (other_width - width)
                    } else {
                        other_code != code >> (width - other_width)
                    });
                }
            }
        }
    }
}
