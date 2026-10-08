#!/usr/bin/env python3
"""Original legacy SBR data syntax vectors, explicit offline generation only.

GOST R53556.4-2013 tables 65/66/69/72/73, ISO 14496-3 SBR syntax.
Uses saved normative Huffman codewords, not a foreign encoder or decoder.
These are bit syntax vectors, not HE-AAC media/PCM acceptance fixtures.
"""
from pathlib import Path
import hashlib
import json

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'tests/fixtures/playback-errors'
BOOKS = json.loads((DEST / 'aac-sbr-huffman-codewords.json').read_text())

def field(value, width):
    assert 0 <= value < 1 << width
    return f'{value:0{width}b}'

def word(book, delta):
    table = BOOKS[book]
    row = table['rows'][table['offset'] + delta]
    return field(row[2], row[1])

def grid(kind):
    # Independently authored syntax and semantic expectations.
    expected = dict(class_name=kind, leading_offset=0, trailing_offset=0,
                    leading_relative=[], trailing_relative=[], pointer=0)
    if kind == 'fixfix1':
        expected['class_name'] = 'FixFix'
        expected['high_resolution'] = [True]
        return '00001', expected
    if kind == 'fixfix2':
        expected['class_name'] = 'FixFix'
        expected['high_resolution'] = [False, False]
        return '00010', expected
    if kind == 'fixvar':
        expected.update(class_name='FixVar', trailing_relative=[2], pointer=2,
                        high_resolution=[True,False])
        return '01'+'00'+'01'+'00'+'10'+'01', expected
    if kind == 'varfix':
        expected.update(class_name='VarFix', leading_relative=[4], pointer=1,
                        high_resolution=[False,True])
        return '10'+'00'+'01'+'01'+'01'+'01', expected
    expected.update(class_name='VarVar', leading_offset=1, trailing_offset=1,
                    leading_relative=[2], trailing_relative=[2], pointer=3,
                    high_resolution=[True,False,True])
    return '11'+'01'+'01'+'01'+'01'+'00'+'00'+'11'+'101', expected

def coefficients(high, flags, amplitude, balance, channel, noise=False):
    text = ''
    rows = []
    for index, temporal in enumerate(flags):
        count = 3 if noise else (6 if high[index] else 3)
        if noise:
            time_book, frequency_book, width = (9,7,5) if balance else (8,5,5)
        else:
            time_book, frequency_book, width = (
                (6,7,5) if amplitude else (2,3,6)
            ) if balance else ((4,5,6) if amplitude else (0,1,7))
        deltas = [((i+index+channel)%3)-1 for i in range(count)]
        if not temporal:
            deltas[0] = (10+channel+index) if noise else (18+channel+index)
            text += field(deltas[0], width)
        text += ''.join(word(time_book if temporal else frequency_book, value)
                        for value in deltas[int(not temporal):])
        rows.append(dict(direction='Time' if temporal else 'Frequency', values=deltas))
    return text, rows

def sample(kind, grid_name, amplitude, temporal, extra, extension_count, slots):
    count = 1 if kind == 'mono' else 2
    coupled = kind == 'coupled'
    text = field(int(extra),1) + ('1010'*count if extra else '')
    if count == 2: text += field(int(coupled),1)
    g0, e0 = grid(grid_name)
    g1, e1 = grid('varfix' if grid_name != 'varfix' else 'fixvar')
    text += g0
    if count == 2 and not coupled: text += g1
    grids = [e0] if count == 1 else [e0, e0 if coupled else e1]
    channels = []
    for ch, g in enumerate(grids):
        n = len(g['high_resolution'])
        env_flags = [bool((i+ch+int(temporal))%2) for i in range(n)]
        noise_flags = [bool((i+ch+int(temporal)+1)%2) for i in range(1 if n==1 else 2)]
        text += ''.join(field(int(f),1) for f in env_flags+noise_flags)
        channels.append(dict(grid=g, envelope_flags=env_flags, noise_flags=noise_flags))
    for ch in range(1 if coupled else count):
        modes = [(ch+i+int(extra))%4 for i in range(3)]
        text += ''.join(field(v,2) for v in modes)
        channels[ch]['inverse_filter'] = modes
    if coupled: channels[1]['inverse_filter'] = channels[0]['inverse_filter']
    envelopes, noises = [], []
    for ch, expected in enumerate(channels):
        g = expected['grid']
        actual_amplitude = amplitude and grid_name != 'fixfix1' if ch == 0 or coupled else amplitude
        env, expected['envelope'] = coefficients(g['high_resolution'], expected['envelope_flags'],
                                                actual_amplitude, coupled and ch==1, ch)
        noise, expected['noise'] = coefficients([], expected['noise_flags'], False,
                                               coupled and ch==1, ch, noise=True)
        envelopes.append(env); noises.append(noise)
    if coupled:
        text += ''.join(e+n for e,n in zip(envelopes,noises))
    else:
        text += ''.join(envelopes) + ''.join(noises)
    for ch, expected in enumerate(channels):
        present = (ch+int(extra))%2 == 0
        harmonics = [(i+ch)%3 == 0 for i in range(6)] if present else [False]*6
        expected['harmonics'] = harmonics
        text += field(int(present),1)
        if present: text += ''.join(field(int(v),1) for v in harmonics)
    text += field(int(extension_count is not None),1)
    extended = None
    if extension_count is not None:
        text += field(min(extension_count,15),4)
        if extension_count >= 15: text += field(extension_count-15,8)
        extended = [(i*73+19)%256 for i in range(extension_count)]
        text += ''.join(field(v,8) for v in extended)
    return text, dict(kind=kind, coupled=coupled, amplitude=amplitude, slots=slots,
                      channels=channels, extended_data=extended)

def main():
    cases = []
    binary = bytearray()
    kinds = ['mono','uncoupled','coupled']
    sizes = [None,0,1,14,15,16,270]
    inputs = []
    for g in ['fixfix1','fixfix2','fixvar','varfix','varvar']:
        for kind in kinds:
            for amp in [False,True]:
                for temporal in [False,True]:
                    i = len(inputs)
                    inputs.append((kind,g,amp,temporal,bool(i%2),sizes[i%len(sizes)],15+i%2))
    for kind in kinds:
        for size in sizes:
            inputs.append((kind,'fixfix1',True,False,True,size,16))
    for args in inputs:
        text, expected = sample(*args)
        padded = text + '0'*((-len(text))%8)
        raw = bytes(int(padded[p:p+8],2) for p in range(0,len(padded),8))
        expected.update(offset=len(binary), bit_length=len(text))
        binary.extend(raw)
        cases.append(expected)
    (DEST/'aac-sbr-data-syntax.bin').write_bytes(binary)
    (DEST/'aac-sbr-data-syntax.json').write_text(json.dumps(dict(
        kind='original bit syntax vectors; not encoded HE-AAC playback acceptance',
        sha256=hashlib.sha256(binary).hexdigest(), vectors=cases), separators=(',',':'))+'\n')
    print(f'{len(cases)} original payloads; {len(binary)} bytes; sha256 {hashlib.sha256(binary).hexdigest()}')

if __name__ == '__main__':
    main()
