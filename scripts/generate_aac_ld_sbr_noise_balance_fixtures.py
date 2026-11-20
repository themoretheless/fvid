"""Owned short stereo noise-balance videos; explicit offline generation.

Uses directly evaluated AAC 4.6.18.3.5 L/R fractions and independent cosine,
CLDFB, HF/gain/noise convolution. No external codec, media or parameter sets.
"""
import json, struct
from pathlib import Path
from generate_aac_eld_fixtures import generate, mkv
from generate_aac_eld_sbr_fixtures import analysis, reconstruct, sbr_bits
from generate_aac_eld_cldfb_fixtures import window, synthesis
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_sbr_dsp_fixtures import header
from generate_he_aac_packet_fixtures import field, frequency, packed, video_fixture
ROOT=Path(__file__).resolve().parents[1]
FIX=ROOT/'tests/fixtures/playback-errors'
OUT=FIX/'aac-ld-sbr-noise-balance'
def main():
    OUT.mkdir(exist_ok=True)
    core=generate(True,[(n,24000,2,'base',0,0,False)for n in [480,512]],frame_count=8)
    noise=[complex(*v)for v in struct.iter_unpack('<dd',(FIX/'aac-sbr-noise-protocol.f64le').read_bytes())]
    c=window();_,high,low,_=tables(10,27,0,False,0,0)
    blob=bytearray();cases=[]
    for base in json.loads(core['manifest.json'])['cases']:
        n=base['n'];pcm=[v[0]for v in struct.iter_unpack('<f',core[base['name']+'.f32le'])]
        for mode in ['coupled','coupled-pan']:
            name=f'n{n}-{mode}'
            # Own coarse wire Q1=8 for pan, Q1=12 for centered. At Q0=7,
            # pan left Q=1/17 and right Q=16/17, not the reverse.
            asc=packed(field(31,5)+field(7,6)+frequency(24000)+field(2,4)+field(n==480,1)+'000111'+header(0,True)+'000000')
            planar=[[v/32768 for v in synthesis(reconstruct(analysis(pcm[ch::2],c),n//32,noise,mode,ch),64,c)]for ch in range(2)]
            gold=b''.join(struct.pack('<ff',*v)for v in zip(*planar))
            packets=[];frames=[]
            for i,r in enumerate(base['frames']):
                wire=packed(r['bits']+sbr_bits(2,len(high)-1,len(low)-1,n//32,i,True,mode))
                frames.append(dict(offset=len(blob),bytes=len(wire)));blob.extend(wire);packets.append(wire)
            case=dict(name=name,n=n,slots=n//32,bands=64,rate=48000,channels=2,mode=mode,asc=asc.hex(),frames=frames,samples=16*n,
                      container_rate=48000,container_frame_samples=2*n,pcm_offset=0,edges=[2*n,6*n,14*n],
                      video=name+'.mp4',file=name+'.mkv',golden=name+'.f32le')
            (OUT/case['golden']).write_bytes(gold)
            (OUT/case['video']).write_bytes(video_fixture([case],blob,2,write=False)['data'])
            (OUT/case['file']).write_bytes(mkv(asc,packets,48000,2*n,2));cases.append(case)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,exports=[dict(c,name=c['name']+'-matroska',video=c['file'])for c in cases]),indent=2)+'\n')
    print('4 authored stereo programmes, MP4 video + Matroska audio controls')
if __name__=='__main__':main()
