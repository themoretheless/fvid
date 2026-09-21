#!/usr/bin/env python3
"""Extract normative H.265 (08/2021) CABAC init tables from pypdf text.
Usage: python3 scripts/generate_hevc_cabac_tables.py STANDARD.txt OUTPUT.rs
The text is the complete, unmodified pypdf page extraction (page markers allowed).
"""
import pathlib
import re
import sys
text = pathlib.Path(sys.argv[1]).read_text()
sizes = {14:2,20:9,21:6,22:15,24:6,25:6,26:54,27:54,28:12,29:132,30:72,31:18}
tables = {}
for number, size in sizes.items():
    matches = list(re.finditer(rf'Table 9-{number} – Values of initValue', text))
    start = matches[-1].end()
    end = text.index(f'Table 9-{number+1} – Values', start)
    rows = re.findall(r'^initValue ([0-9 ]+)\s*$', text[start:end], re.M)
    values = [int(value) for row in rows for value in row.split()]
    if len(values) != size:
        raise ValueError(f'table {number}: {len(values)} values, expected {size}')
    tables[number] = values
if tables[26] != tables[27]: raise ValueError('last-X/Y tables unexpectedly differ')
output = ['//! Generated from ITU-T H.265 (08/2021), tables 9-14 and 9-20..31.',
          '//! Regenerate with scripts/generate_hevc_cabac_tables.py; values are in normative ctxIdx order.']
for number, values in tables.items():
    output.append(f'pub(super) const TABLE_{number}: [u8; {len(values)}] = {values};')
pathlib.Path(sys.argv[2]).write_text('\n'.join(output)+'\n')
