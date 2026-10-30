"""Authored AAC finite edit windows with independent native-grid phase."""
from pathlib import Path
import json,struct,hashlib
from generate_mp4_aac_quicktime_regressions import generate,boxes,atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-aac-edit-phase'
def main():
    own=generate();files={'control.mp4':own['iso-v0-direct.mp4'],'reference.f32le':own['expected.f32le']};cases=[]
    for factor in [4,40,65537]:
        for start in [0,1,factor]:
            for shift in [-1,0,1]:
                def rewrite(data,audio=False):
                    out=b''
                    for k,b in boxes(data):
                        selected=audio
                        if k==b'trak':selected=dict(boxes(dict(boxes(b))[b'mdia']))[b'hdlr'][8:12]==b'soun'
                        if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                            b=rewrite(b,selected)
                            if k==b'stbl' and audio:b+=atom(b'ctts',word(1<<24,1,3)+struct.pack('>i',shift))
                            if k==b'trak' and selected:b+=atom(b'edts',atom(b'elst',word(0,1,2048,start,65536)))
                        elif k==b'mdhd' and audio:b=b[:12]+word(48000*factor,3072*factor)+b[20:]
                        elif k==b'stts' and audio:b=word(0,1,3,1024*factor)
                        elif k==b'tkhd' and audio:b=b[:20]+word(2048)+b[24:]
                        out+=atom(k,b)
                    return out
                name=f'factor{factor}-start{start}-shift{shift}.mp4';files[name]=rewrite(own['iso-v0-direct.mp4']);first=max(0,(start-shift+factor-1)//factor)
                cases.append(dict(file=name,factor=factor,start=start,shift=shift,first=first,last=(start+2048*factor-shift+factor-1)//factor,phase=first*factor+shift-start))
    for c in list(cases):
        start,factor=c['start'],c['factor']
        def split(data):
            out=b''
            for k,b in boxes(data):
                if k in [b'moov',b'trak',b'edts']:b=split(b)
                elif k==b'elst':b=word(0,2,1024,start,65536,1024,start+1024*factor,65536)
                out+=atom(k,b)
            return out
        name='split-'+c['file'];files[name]=split(files[c['file']]);cases.append(dict(c,file=name))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own AAC-LC pulse/overlap and analytic PCM with neutral VP9; a finite 2048-native-sample unit-rate edit or two contiguous equivalent edits at exact media origins, signed sub-sample CTTS, 4x/40x/65537x media clocks. No private media/parameters, external encoder, FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
