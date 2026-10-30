#!/usr/bin/env python3
"""Own VP9 fMP4 init segments with one open-ended rate-one media edit."""
from pathlib import Path
import struct,json,hashlib
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/mp4-open-ended-edit'


def generate():
    packet=frame(8,'420',16,16)
    fragment=(ROOT/'tests/fixtures/playback-errors/mp4-tfhd-defaults/tfhd-overrides-trex-nonsync.mp4').read_bytes()
    regular=mp4([packet]*3,8,'420',(16,16),[1,2,3])
    def shifted(data,shift):
        out=b''
        for k,b in atoms(data):
            if k in [b'moof',b'traf']:b=shifted(b,shift)
            elif k==b'trun':
                flags,count,offset=struct.unpack('>III',b);assert flags==1 and count==3
                b=word(flags|0x800,count,offset+12)+word(*([shift]*3))
            out+=atom(k,b)
        return out
    def build(base,version,scale,duration,start,rate=0x10000,multiple=False):
        row=word(duration,start&0xffffffff,rate) if version==0 else struct.pack('>QqI',duration,start,rate)
        elst=word(version<<24,2 if multiple else 1)+row*(2 if multiple else 1)
        def rewrite(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak']:
                    b=rewrite(b)
                    if k==b'trak':b+=atom(b'edts',atom(b'elst',elst))
                elif k==b'tkhd':
                    b=b[:20]+word(duration)+b[24:]
                elif k==b'mvhd':
                    b=b[:12]+word(scale,duration)+b[20:]
                out+=atom(k,b)
            return out
        return rewrite(base)
    files={'frame.vp9':packet};controls=[];opened=[];invalid=[]
    for version in [0,1]:
        for scale in [50,1000,1001]:
            for profile,start,shift in [('whole',0,0),('trim',1,0),('positive-cts',2,2)]:
                base=shifted(fragment,shift) if shift else fragment
                frames=3+shift-start
                for is_open in [False,True]:
                    duration=0 if is_open else frames*scale//50
                    n=f'v{version}-clock{scale}-{profile}-'+('open.mp4' if is_open else 'known.mp4')
                    files[n]=build(base,version,scale,duration,start)
                    (opened if is_open else controls).append({'file':n,'frames':frames,'start':start,'shift':shift,'duration':duration,'movie_scale':scale})
        # A 20ms remaining frame cannot be encoded as a positive movie duration
        # with a 25Hz clock. An open edit must resolve in media time, not round.
        n=f'v{version}-clock25-single-open.mp4';files[n]=build(fragment,version,25,0,2)
        opened.append({'file':n,'frames':1,'start':2,'shift':0,'duration':0,'movie_scale':25})
        for label,base,start,rate,multiple in [('regular',regular,0,0x10000,False),('empty',fragment,-1,0x10000,False),('rate-zero',fragment,0,0,False),('multiple',fragment,0,0x10000,True)]:
            n=f'v{version}-{label}-invalid.mp4';files[n]=build(base,version,50,0,start,rate,multiple)
            invalid.append(n)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Pure-Python owned 16x16 neutral VP9. Three media samples at 50Hz; single media edit V0/V1, clocks 25/50/1000/1001, trimmed starts and positive composition shifts. No private source, FFmpeg or network.','controls':controls,'open':opened,'invalid':invalid,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')

if __name__=='__main__':generate()
