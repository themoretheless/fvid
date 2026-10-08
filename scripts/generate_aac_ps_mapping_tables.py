#!/usr/bin/env python3
"""Offline numeric PS grid/routing tables from saved normative geometry.

No foreign codec implementation, FFmpeg or network. Algorithms remain owned
Rust; these are only mandatory weights and integer/boolean routing data.
"""
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]

def main():
    data=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ps-mapping-protocol.json').read_text())
    lines=['//! Numeric protocol geometry: GOST R 53556.8-2013 tables 45/46/48/49.',
           '//! Generated explicitly/offline by generate_aac_ps_mapping_tables.py.',
           'use super::aac_ps_mapping::Binding;']
    for key,name,length,source in [('up20_to34','UP',34,20),('down34_to20','DOWN',20,34)]:
        rows=data[key];assert len(rows)==length
        lines.append(f'pub(super) const {name}: &[(&[(u8, u8)], u8)] = &[')
        for row in rows:
            assert 1<=len(row['terms'])<=4
            assert sum(w for _,w in row['terms'])==row['denominator']
            assert all(0<=i<source and w>0 for i,w in row['terms'])
            terms=', '.join(f'({i}, {w})' for i,w in row['terms'])
            lines.append(f'    (&[{terms}], {row["denominator"]}),')
        lines.append('];')
    for key,name,length,bands in [('hybrid20','HYBRID_20',71,20),('hybrid34','HYBRID_34',91,34)]:
        rows=data[key];assert len(rows)==length
        lines.append(f'pub(super) const {name}: &[Binding] = &[')
        for row in rows:
            assert 0<=row['qmf']<64 and 0<=row['parameter']<bands
            conjugate='true' if row['conjugate'] else 'false'
            lines += ['    Binding {',f'        qmf: {row["qmf"]},',f'        parameter: {row["parameter"]},',
                      f'        conjugate: {conjugate},','    },']
        lines.append('];')
    (ROOT/'crates/fvid-media/src/owned_aac/aac_ps_mapping_tables.rs').write_text('\n'.join(lines)+'\n')

if __name__=='__main__':main()
