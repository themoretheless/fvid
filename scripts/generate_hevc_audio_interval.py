#!/usr/bin/env python3
"""Dedicated interval refusals/acceptance on saved authored HEVC/AAC/Opus."""
import json,hashlib
from generate_hevc_matroska_formats import D

def main():
    cases=[]
    original=json.loads((D/'hevc-matroska-reorder.json').read_text())['cases']
    for index,c in enumerate(c for c in original if c['companions']):
        data=(D/c['file']).read_bytes();file=f'hevc-audio-interval-source-{index}.mkv';(D/file).write_bytes(data)
        cases.append(dict(c,source_file=c['file'],file=file,intervals_us=[[0,91000],[17123,91789],[67000,119999]],sha256=hashlib.sha256(data).hexdigest()))
    assert len(cases)==8
    (D/'hevc-audio-interval.json').write_text(json.dumps(dict(cases=cases,provenance='Dedicated short interval reproducers reuse only saved authored IPB HEVC and synthetic AAC/Opus packets. Boundaries cross sample/frame positions. No private media, codec execution, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
