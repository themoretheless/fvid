#!/usr/bin/env python3
"""Equivalent authored CTTS run layouts across legacy MACE compression blocks."""
import hashlib
import json
import struct
from pathlib import Path
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom, word
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-mace-composition-runs'

def main():
    source = OUT.parent / 'mp4-mace-transitions'
    files = {}; pairs = []; damaged = []
    def rewrite(data, rows, version):
        def walk(data, audio=False):
            out = b''
            for kind, body in atoms(data):
                if kind == b'trak': body = walk(body, b'soun' in body)
                elif kind in [b'moov', b'mdia', b'minf', b'stbl']:
                    body = walk(body, audio)
                    if kind == b'stbl' and audio:
                        payload = word(version << 24, len(rows))
                        for count, offset in rows: payload += struct.pack('>II', count, offset & 0xffffffff)
                        body += atom(b'ctts', payload)
                out += atom(kind, body)
            return out
        return walk(data)
    for name, count in [('control-0',240),('control-1',240),('control-2',240),('control-6mono',240),('mixed',1200)]:
        original = (source / (name+'.mp4')).read_bytes()
        for label, version, shift in [('zero',0,0),('positive',0,36),('negative',1,-36)]:
            control=f'{name}-{label}-grouped.mp4'; split=f'{name}-{label}-split.mp4'
            files[control]=rewrite(original,[(count,shift)],version)
            files[split]=rewrite(original,[(1,shift),(2,shift),(3,shift),(count-6,shift)],version)
            pairs.append({'control':control,'split':split,'version':version,'raw_samples':count,'offset':shift})
        for label, rows in [('changed-inside-block',[(1,0),(count-1,1)]),('short',[(count-1,0)]),('zero-row',[(0,0),(count,0)]),('excess-count',[(1,0),(count,0)])]:
            path=f'bad-{name}-{label}.mp4';files[path]=rewrite(original,rows,0)
            damaged.append({'file':path,'expected':'MACE composition run must cover whole blocks'})
    for name,count in [('control-0',240),('mixed',1200)]:
        original=(source/(name+'.mp4')).read_bytes()
        control=f'{name}-changing-grouped.mp4';split=f'{name}-changing-split.mp4'
        files[control]=rewrite(original,[(6,0),(count-6,36)],0)
        files[split]=rewrite(original,[(1,0),(5,0),(2,36),(count-8,36)],0)
        pairs.append({'control':control,'split':split,'version':0,'raw_samples':count,'offset':'changes after first whole block'})
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Earlier authored arithmetic MACE bytes and neutral16x16 VP9 only; common CTTS layouts are newly authored, no private media/parameters, encoder or network.','reference':'https://developer.apple.com/documentation/quicktime-file-format/composition_offset_atom/entry_count','pairs':pairs,'damaged':damaged,'source_sha256':{n:hashlib.sha256((source/(n+'.mp4')).read_bytes()).hexdigest() for n in ['control-0','control-1','control-2','control-6mono','mixed']},'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__ == '__main__':main()
