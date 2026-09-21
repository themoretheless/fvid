#!/usr/bin/env python3
"""Regenerate numeric CAVLC tables from ITU-T H.264 (02/2016).

Requires pdfplumber; no codec code is copied. Source PDF:
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S!!PDF-E&lang=e&type=items
Run: python3 scripts/generate_cavlc_tables.py /path/to/standard.pdf
Then: rustfmt --edition 2024 src/codec/cavlc_tables.rs
"""
import hashlib
import sys
from pathlib import Path
import pdfplumber

source = Path(sys.argv[1])
assert hashlib.sha256(source.read_bytes()).hexdigest() == '1a73f86fd39fb00e2b15dc543c1a18f8f49c012dd76876f16b3669f0ae6d9ab2', 'Unexpected PDF revision; re-audit table page indices first'
with pdfplumber.open(source) as pdf:
    pages = {str(i): pdf.pages[i].extract_tables() for i in [239,240,241,244,245,246]}

def code(bits,value):
    bits=''.join((bits or '').split())
    if bits in ('','-'):return None
    assert set(bits)<=set('01'),bits
    return (int(bits,2),len(bits),value)

def verify(entries):
    assert len({(b,n) for b,n,_ in entries})==len(entries)
    for a,na,_ in entries:
        for b,nb,_ in entries:
            if na<nb:assert b>>(nb-na)!=a,(a,na,b,nb)
    return entries

tokens=[[] for _ in range(6)]
for page in ['239','240','241']:
    for row in pages[page][0][1:]:
        trailing,total=map(int,row[:2]);assert trailing<=min(3,total)
        for c,bits in enumerate(row[2:]):
            entry=code(bits,total*4+trailing)
            if entry:tokens[c].append(entry)
assert [len(x) for x in tokens]==[62,62,62,62,14,30]

def columns(table):
    result=[[] for _ in table[1][1:]]
    for row in table[2:]:
        value=int(row[0])
        for c,bits in enumerate(row[1:]):
            entry=code(bits,value)
            if entry:result[c].append(entry)
    return result

groups={'COEFF_TOKEN':tokens,'TOTAL_ZEROS':columns(pages['244'][0])+columns(pages['244'][1]),
'CHROMA_420_ZEROS':columns(pages['245'][0]),'CHROMA_422_ZEROS':columns(pages['245'][1]),
'RUN_BEFORE':columns(pages['246'][0])}
out=['// Numeric VLC mappings from ITU-T H.264 (02/2016), Tables 9-5 and 9-7 through 9-10.\n// Tuple: code bits, bit length, decoded value. coeff_token value = TotalCoeff * 4 + TrailingOnes.\npub(super) type Code = (u16, u8, u8);\n']
for name,tables in groups.items():
    out.append(f'pub(super) const {name}: &[&[Code]] = &[\n')
    for table in tables:
        verify(table)
        out.append('    &['+','.join(f'({b},{n},{v})' for b,n,v in table)+'],\n')
    out.append('];\n')
(Path(__file__).resolve().parents[1] / 'src/codec/cavlc_tables.rs').write_text(''.join(out))
print({name:[len(t) for t in tables] for name,tables in groups.items()})
