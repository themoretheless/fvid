#!/usr/bin/env python3
"""Short authored VP9/IMA4 payloads at wide fragmented audio timestamps."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_audio_composition_order import generate as controls
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-audio-wide-time'
def generate():
    seed=controls();source=seed['ordered.mp4'];parts=dict(atoms(source))
    def init(body,audio=False):
        out=b''
        for k,b in atoms(body):
            if k==b'trak':b=init(b,b'soun' in b)
            elif k in [b'mdia',b'minf',b'stbl']:b=init(b,audio)
            elif audio and k in [b'stts',b'stsc',b'stco']:b=word(0,0)
            elif audio and k==b'stsz':b=word(0,0,0)
            elif audio and k==b'mdhd':b=b[:16]+word(0)+b[20:]
            out+=atom(k,b)
        return out
    movie=init(parts[b'moov'])+atom(b'mvex',atom(b'trex',word(0,2,1,64,34,0)))
    prefix=atom(b'ftyp',parts[b'ftyp'])+atom(b'mdat',parts[b'mdat'])+atom(b'moov',movie)
    payload=b''.join(seed[f'packet-{i}.ima4']for i in range(3))
    def fragment(start,offset):
        traf=atom(b'tfhd',word(0x020018,2,64,34))+atom(b'tfdt',word(1<<24)+struct.pack('>Q',start))+atom(b'trun',word(1,3,offset))
        return atom(b'moof',atom(b'mfhd',word(0,1))+atom(b'traf',traf))
    limit=(1<<64)-1;seconds=limit//1_000_000_000
    files={'expected.f32le':seed['ordered.f32le']};cases=[]
    for label,start,valid in [('control',0,True),('below-nanosecond-limit',(seconds-1)*8000,True),('above-nanosecond-limit',(seconds+1)*8000,True),('near-signed-tick-limit',(1<<63)-256,True),('signed-overflow',1<<63,False)]:
        moof=fragment(start,0);name=label+'.mp4';files[name]=prefix+fragment(start,len(moof)+8)+atom(b'mdat',payload)
        cases.append({'file':name,'start':start,'end':start+192,'valid':valid})
    files['manifest.json']=(json.dumps({'provenance':'Authored neutral16x16 VP9 three frames60ms and analytic fresh-predictor IMA4 three blocks24ms. tfdt V1 shifts audio coordinates only; no long media allocation,private input,encoder or network.','rate':8000,'cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
