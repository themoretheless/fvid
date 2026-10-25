"""Owned neutral-video E-AC-3 multiplexes on quantized media clocks."""
from pathlib import Path
import json,struct
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word
from generate_mp4_quantized_audio_clock import quantize
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-eac3-programme-clock'
SOURCE=OUT.parent/'eac3-mp4-programmes'
def variable_packets(data,audio=False):
    result=b''
    for k,b in atoms(data):
        if k==b'trak':b=variable_packets(b,b'soun'in b)
        elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=variable_packets(b,audio)
        elif audio and k==b'stsz':
            fixed,count=struct.unpack_from('>II',b,4);assert count==3
            sizes=[fixed]*count if fixed else list(struct.unpack_from('>III',b,12))
            assert len(set(sizes))==1 and sizes[0]%2==0
            n=sizes[0]//2;b=bytes(4)+word(0,3,n,3*n,2*n)
        elif audio and k==b'stts':b=bytes(4)+word(3,1,768,1,2304,1,1536)
        result+=atom(k,b)
    return result
def generate():
    original=json.loads((SOURCE/'manifest.json').read_bytes());files={};cases=[]
    for family in ['dual-48000','heterogeneous-44100','expanded-48000-b3','expanded-44100-b6','51-32000','placeholder-rate','heterogeneous-44100-variable']:
        variable=family.endswith('-variable');base=family.removesuffix('-variable')
        record=next(c for c in original['cases']if c['name']==base)
        data=(SOURCE/(base+'.mp4')).read_bytes()
        if variable:data=variable_packets(data)
        files[family+'-exact.mp4']=data
        for p,control in enumerate(record['controls']):
            raw=(SOURCE/(control['name']+'.mp4')).read_bytes();files[control['name']+('-variable' if variable else '')+'.mp4']=variable_packets(raw) if variable else raw
        for scale in [900,1000,1001]:
            for mode in ['floor','ceil','nearest']:
                name=f'{family}-{scale}-{mode}.mp4';files[name]=quantize(data,scale,mode)
                for p,control in enumerate(record['controls']):cases.append(dict(file=name,programme=p,control=control['name']+('-variable' if variable else '')+'.mp4',rate=record['rate'],channels=control['channels'],samples_per_packet=[768,2304,1536] if variable else [1536]*3))
    for p in [1,7]:
        data=(SOURCE/f'corrupt-unselected-p{p}.mp4').read_bytes()
        for scale in [900,1000,1001]:
            for mode in ['floor','ceil','nearest']:
                name=f'corrupt-unselected-p{p}-{scale}-{mode}.mp4';files[name]=quantize(data,scale,mode)
                cases.append(dict(file=name,programme=p,control=f'dual-48000-p{p}-control.mp4',rate=48000,channels=3,samples_per_packet=[1536]*3))
    files['without-programme7-900.mp4']=quantize((SOURCE/'without-programme7.mp4').read_bytes(),900,'floor')
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Existing repository-authored neutral VP9 and authored E-AC-3 independent/dependent spectra from eac3-mp4-programmes. No private media, process, FFmpeg or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
