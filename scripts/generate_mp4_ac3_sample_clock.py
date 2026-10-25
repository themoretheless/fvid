"""Owned neutral video + existing authored AC-3 tone, cumulative coarse clocks."""
from pathlib import Path
import json
from generate_ac3_mp4_channels import generate as channels
from generate_mp4_quantized_audio_clock import quantize
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-ac3-sample-clock'
def generate():
    original=channels();source=original['native-six.mp4'];size=len(original['control.ac3'])//3
    def group(data,audio=False,all_in_one=False):
        result=b''
        for k,b in atoms(data):
            if k==b'trak':b=group(b,b'soun'in b,all_in_one)
            elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=group(b,audio,all_in_one)
            elif audio and k==b'stsz':b=bytes(4)+(word(size*3,1) if all_in_one else word(0,2,size,size*2))
            elif audio and k==b'stts':b=bytes(4)+(word(1,1,4608) if all_in_one else word(2,1,1536,1,3072))
            elif audio and k==b'stsc':b=bytes(4)+word(1,1,1 if all_in_one else 2,1)
            result+=atom(k,b)
        return result
    files={'exact.mp4':source,'grouped-exact.mp4':group(source),'bad-first-exact.mp4':original['bad-first-header.mp4']};files['one-packet-exact.mp4']=group(source,all_in_one=True);files['one-packet-bad-first-exact.mp4']=group(original['bad-first-header.mp4'],all_in_one=True);cases=[]
    for family,control in [('ac3','exact.mp4'),('grouped','grouped-exact.mp4'),('bad-first','bad-first-exact.mp4'),('one-packet','one-packet-exact.mp4'),('one-packet-bad-first','one-packet-bad-first-exact.mp4')]:
        for scale in [900,1000,1001]:
            for mode in ['floor','ceil','nearest']:
                name=f'{family}-{scale}-{mode}.mp4'
                files[name]=quantize(files[control],scale,mode)
                cases.append(dict(file=name,exact=control,scale=scale,rounding=mode,samples_per_packet=[4608] if family.startswith("one-packet") else [1536,3072] if family=="grouped" else [1536]*3))
    files['manifest.json']=(json.dumps(dict(cases=cases,rate=48000,channels=6,syncframes=3,native_samples=4608,provenance='Repository-authored short synthetic tone and neutral VP9 video, reused from ac3-mp4-channels. No private data, subprocess, network or FFmpeg.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
