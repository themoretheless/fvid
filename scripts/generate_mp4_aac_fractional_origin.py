"""Authored AAC-LC/VP9 with signed sub-sample composition origins."""
from pathlib import Path
import struct,json,hashlib
from generate_mp4_aac_quicktime_regressions import generate,boxes,atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-aac-fractional-origin'
def main():
    own=generate();files={'control.mp4':own['iso-v0-direct.mp4'],'reference.f32le':own['expected.f32le']};cases=[]
    for factor in [4,40,65537]:
        for shift in [-5,-1,0,1,5]:
            def rewrite(data,audio=False):
                result=b''
                for k,b in boxes(data):
                    selected=audio
                    if k==b'trak':selected=dict(boxes(dict(boxes(b))[b'mdia']))[b'hdlr'][8:12]==b'soun'
                    if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                        b=rewrite(b,selected)
                        if k==b'stbl' and audio:b+=atom(b'ctts',word(1<<24,1,3)+struct.pack('>i',shift))
                    elif k==b'mdhd' and audio:b=b[:12]+word(48000*factor,3072*factor)+b[20:]
                    elif k==b'stts' and audio:b=word(0,1,3,1024*factor)
                    elif k==b'mvhd':b=b[:16]+word(3072+max(0,(shift+factor-1)//factor))+b[20:]
                    elif k==b'tkhd' and audio:b=b[:20]+word(3072+(shift+factor-1)//factor)+b[24:]
                    result+=atom(k,b)
                return result
            name=f'factor{factor}-shift{shift}.mp4';files[name]=rewrite(own['iso-v0-direct.mp4']);cases.append(dict(file=name,factor=factor,shift=shift,scale=48000*factor,drop=max(0,(-shift+factor-1)//factor)))
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own 64ms 48kHz mono AAC-LC pulse/overlap, analytic IMDCT reference and neutral16x16 VP9; exact 4x/40x/65537x media clocks with signed CTTS smaller than or near one native sample. No private media/parameters, encoder, FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
