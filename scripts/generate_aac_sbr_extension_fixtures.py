#!/usr/bin/env python3
"""Original SBR outer framing vectors; no media decoder/encoder required.

Uses saved original SCE/CPE syntax bits. CRC oracle is GF(2) long division
of the whole message polynomial times x^10, independent of Rust's register.
Published MPEG-4 SBR CRC polynomial: x^10+x^9+x^5+x^4+x+1, initial zero.
These are syntax/CRC vectors, not encoded HE-AAC PCM acceptance fixtures.
"""
from pathlib import Path
import hashlib
import json

DEST = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'

def crc(text):
    value = int(text,2) << 10
    while value.bit_length() > 10:
        value ^= 0x633 << (value.bit_length()-11)
    return value

def header(amplitude):
    # start=0, stop=14, crossover=0, reserved=0, extra1=1, extra2=0;
    # scale=0, alter=0, noise=3. Other controls default from absent extra2.
    return str(int(amplitude))+'0000'+'1110'+'000'+'00'+'10'+'00'+'0'+'11'

def main():
    source = (DEST/'aac-sbr-data-syntax.bin').read_bytes()
    cases = json.loads((DEST/'aac-sbr-data-syntax.json').read_text())['vectors']
    blob = bytearray(); vectors = []
    for source_index, case in enumerate(cases):
        start = case['offset']; length = case['bit_length']
        data = ''.join(f'{b:08b}' for b in source[start:start+(length+7)//8])[:length]
        for protected in [False,True]:
            initial = len(vectors)
            for present in [True,False]:
                body = str(int(present)) + (header(case['amplitude']) if present else '') + data
                fill_count = (-(4+10*int(protected)+len(body)))%8
                # Nonzero fill values ensure the oracle covers padding bits.
                body += ('1011010'[:fill_count])
                checksum = crc(body) if protected else None
                text = f'{14 if protected else 13:04b}' + (f'{checksum:010b}' if protected else '') + body
                if len(text) > 269*8: continue
                raw = bytes(int(text[p:p+8],2) for p in range(0,len(text),8))
                vectors.append(dict(source_index=source_index, offset=len(blob), byte_length=len(raw),
                                    crc=checksum, header_present=present,
                                    preceding_header=initial if not present else None,
                                    fill_count=fill_count))
                blob.extend(raw)
    (DEST/'aac-sbr-extension-syntax.bin').write_bytes(blob)
    (DEST/'aac-sbr-extension-syntax.json').write_text(json.dumps(dict(
        kind='original framing and polynomial-division CRC vectors, not PCM acceptance',
        sha256=hashlib.sha256(blob).hexdigest(), vectors=vectors),separators=(',',':'))+'\n')
    print(f'{len(vectors)} original extensions, {len(blob)} bytes, sha256 {hashlib.sha256(blob).hexdigest()}')

if __name__ == '__main__': main()
