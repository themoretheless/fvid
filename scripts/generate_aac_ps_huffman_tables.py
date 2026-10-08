#!/usr/bin/env python3
"""Generate Rust PS protocol tables from checked-in normative numeric words.

Offline explicit generation only; no foreign decoder, FFmpeg or network.
"""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'tests/fixtures/playback-errors/aac-ps-huffman-codewords.json'

def main():
    books = json.loads(SOURCE.read_text())['books']
    lines = ['//! Normative PS protocol words, GOST R 53556.8-2013 B.17-B.21.',
             '//! Generated offline by scripts/generate_aac_ps_huffman_tables.py.',
             '//! Only numeric protocol data; decoder algorithms are FVid-owned.']
    for index, book in enumerate(books):
        rows = [(symbol, len(word), int(word, 2)) for symbol, word in book['rows']]
        assert len(set(symbol for symbol, _, _ in rows)) == len(rows)
        maximum = max(width for _, width, _ in rows)
        assert sum(1 << (maximum-width) for _, width, _ in rows) == 1 << maximum
        for i, (_, width, code) in enumerate(rows):
            for _, other_width, other_code in rows[i+1:]:
                assert (code != other_code >> (other_width-width) if width <= other_width
                        else other_code != code >> (width-other_width))
        lines += [f'// {book["name"]}', f'pub(super) const BOOK_{index}: &[(i16, u8, u32)] = &[']
        lines += [f'    ({symbol}, {width}, 0x{code:x}),' for symbol, width, code in rows]
        lines.append('];')
    (ROOT / 'crates/fvid-media/src/owned_aac/aac_ps_huffman_tables.rs').write_text('\n'.join(lines)+'\n')

if __name__ == '__main__':
    main()
