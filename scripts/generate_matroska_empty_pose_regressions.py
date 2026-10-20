#!/usr/bin/env python3
"""Authored VP9 clips with legal empty projection pose defaults (RFC 8794)."""
import json
import hashlib
from pathlib import Path
from generate_matroska_clock_regressions import clip
from generate_audio_resample_window_fixture import ebml

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/matroska-empty-pose'
MASTERS = {0x18538067, 0x1654ae6b, 0xae, 0xe0}

def rewrite(data, pose):
    result = bytearray()
    at = 0
    while at < len(data):
        start = at
        width = 9 - data[at].bit_length()
        tag = data[at:at+width]
        at += width
        width = 9 - data[at].bit_length()
        length = int.from_bytes(data[at:at+width], 'big') & ((1 << (width*7))-1)
        at += width
        payload = data[at:at+length]
        assert len(payload) == length
        at += length
        ident = int.from_bytes(tag, 'big')
        if ident in MASTERS:
            payload = rewrite(payload, pose)
            if ident == 0xe0:
                payload += ebml('7670', pose)
            result += ebml(tag.hex(), payload)
        else:
            result += data[start:at]
    return bytes(result)

def generate():
    source = clip().replace(ebml('4287', b'\3'), ebml('4287', b'\4'))
    cases = {'control.mkv': source}
    for name, tag in [('yaw','7673'), ('pitch','7674'), ('roll','7675')]:
        cases[name+'.mkv'] = rewrite(source, ebml(tag, b''))
    cases['all.mkv'] = rewrite(source, b''.join(ebml(tag,b'') for tag in ['7673','7674','7675']))
    cases['invalid-one-byte.mkv'] = rewrite(source, ebml('7675',b'\0'))
    return cases

def main():
    OUT.mkdir(parents=True,exist_ok=True)
    cases=generate()
    for name,data in cases.items(): (OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored three VP9 keyframes; no external encoder or private media.', 'source':'https://www.rfc-editor.org/rfc/rfc8794.html#section-7.3', 'files':{name:hashlib.sha256(data).hexdigest() for name,data in cases.items()}},indent=2)+'\n')
if __name__ == '__main__': main()
