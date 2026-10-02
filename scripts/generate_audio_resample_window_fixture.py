#!/usr/bin/env python3
"""Synthetic raw ALAC in MP4/Matroska for resampler right-edge lookahead.
No private media, external encoders, parameter sets or reference tools.
"""
from pathlib import Path
import struct
ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
def atom(kind, payload):
    return struct.pack('>I4s', len(payload)+8, kind) + payload
def word(*values):
    return b''.join(struct.pack('>I', v) for v in values)
def ebml(tag, payload):
    return bytes.fromhex(tag) + struct.pack('>I', len(payload) | 0x10000000) + payload
def packet(values):
    bits = '000' + '0000' + '0'*12 + '1' + '00' + '1' + f'{len(values):032b}'
    bits += ''.join(f'{v & 65535:016b}' for v in values) + '111'
    bits += '0' * (-len(bits) % 8)
    return int(bits,2).to_bytes(len(bits)//8,'big')
def generate():
    cookie = bytearray(24)
    cookie[:4] = word(128)
    cookie[5:10] = bytes([16,40,10,14,1])
    cookie[20:24] = word(48000)
    values = [(i*119)%24001-12000 for i in range(512)]
    packets = [packet(values[i:i+128]) for i in range(0,len(values),128)]
    ftyp = atom(b'ftyp', b'isom'+word(0)+b'isommp42')
    mdat = atom(b'mdat', b''.join(packets))
    entry = bytearray(28)
    entry[6:8] = (1).to_bytes(2,'big')
    entry[16:20] = struct.pack('>HH',1,16)
    entry[24:28] = word(48000<<16)
    stsd = atom(b'stsd', word(0,1)+atom(b'alac', entry+atom(b'alac', word(0)+cookie)))
    stbl = atom(b'stbl', stsd+atom(b'stts', word(0,1,4,128))+atom(b'stsz', word(0,0,4,*map(len,packets)))+atom(b'stsc',word(0,1,1,4,1))+atom(b'stco',word(0,1,len(ftyp)+8)))
    dinf = atom(b'dinf', atom(b'dref', word(0,1)+atom(b'url ',word(1))))
    tkhd = bytearray(84)
    tkhd[12:16] = word(1)
    tkhd[20:24] = word(512)
    tkhd[40:44] = tkhd[56:60] = word(65536)
    tkhd[72:76] = word(0x40000000)
    mdhd = bytearray(24)
    mdhd[12:20] = word(48000,512)
    mdhd[20:22] = (21956).to_bytes(2,'big')
    mdia = atom(b'mdia', atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',dinf+stbl))
    mvhd = bytearray(100)
    mvhd[12:20] = word(48000,512)
    mp4 = ftyp+mdat+atom(b'moov',atom(b'mvhd',mvhd)+atom(b'trak',atom(b'tkhd',tkhd)+mdia))
    audio = ebml('e1',ebml('b5',struct.pack('>d',48000))+ebml('9f',b'\x01')+ebml('6264',b'\x10'))
    track = ebml('ae',ebml('d7',b'\x01')+ebml('83',b'\x02')+ebml('86',b'A_ALAC')+ebml('63a2',cookie)+audio)
    segment = ebml('1549a966',ebml('2ad7b1',b'\x01'))+ebml('1654ae6b',track)
    for i,payload in enumerate(packets):
        pts = (i*128*1000000000+24000)//48000
        segment += ebml('1f43b675',ebml('e7',pts.to_bytes(8,'big'))+ebml('a3',b'\x81\0\0\x80'+payload))
    mka = ebml('1a45dfa3',ebml('4282',b'matroska'))+ebml('18538067',segment)
    ROOT.mkdir(parents=True,exist_ok=True)
    (ROOT/'alac-resample-window.m4a').write_bytes(mp4)
    (ROOT/'alac-resample-window.mka').write_bytes(mka)
    precise = [v/32768.0+(i%5)*1e-10 for i,v in enumerate(values)]
    audio64 = ebml('e1',ebml('b5',struct.pack('>d',48000))+ebml('9f',b'\x01')+ebml('6264',b'\x40'))
    track64 = ebml('ae',ebml('d7',b'\x01')+ebml('83',b'\x02')+ebml('86',b'A_PCM/FLOAT/IEEE')+audio64)
    segment64 = ebml('1549a966',ebml('2ad7b1',b'\x01'))+ebml('1654ae6b',track64)
    for i in range(4):
        pts = (i*128*1000000000+24000)//48000
        payload = b''.join(struct.pack('<d',v) for v in precise[i*128:(i+1)*128])
        segment64 += ebml('1f43b675',ebml('e7',pts.to_bytes(8,'big'))+ebml('a3',b'\x81\0\0\x80'+payload))
    (ROOT/'pcm64-resample-window.mka').write_bytes(ebml('1a45dfa3',ebml('4282',b'matroska'))+ebml('18538067',segment64))

if __name__ == '__main__':
    generate()
