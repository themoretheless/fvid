"""Owned E-AC-3 backward-compatible BSID acceptance videos.
Only authoring functions are imported; no generated inputs or external tools.
"""
import json, struct
from pathlib import Path
from generate_eac3_playback import build_blksw_frame, build_spx_atten_frame, patch_crc
from generate_ffv1_gray_fixtures import element, uint
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-bsid'
def identify(raw,bsid):
    raw=bytearray(raw)
    # E.1.2 syncinfo: bsid follows 16+2+3+11+2+2+3+1 = 40 bits.
    raw[5]=(raw[5]&7)|(bsid<<3)
    return patch_crc(raw)
def movie(packets,blocks=6,channels=2,rate=48000,codec="A_EAC3"):
    video=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+element(0xe0,uint(0xb0,16)+uint(0xba,16))
    audio=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,codec.encode())+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,channels))
    vp9=frame(8,'420',16,16)
    clusters=b''.join(element(0x1f43b675,uint(0xe7,i*blocks*256*1000//rate)+element(0xa3,b'\x81\0\0\x80'+vp9)+element(0xa3,b'\x82\0\0\x80'+p)) for i,p in enumerate(packets))
    return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',len(packets)*blocks*256*1000/rate)))+element(0x1654ae6b,element(0xae,video)+element(0xae,audio))+clusters)
def generate():
    files={};cases=[];invalid=[]
    patterns={'long':[(0,0)]*6,'short':[(1,1)]*6,'mixed':[(0,1),(1,0),(1,1),(0,0),(1,0),(0,1)]}
    sources={n:build_blksw_frame(p) for n,p in patterns.items()}
    sources.update({'spx':build_spx_atten_frame(None),'spx-atten':build_spx_atten_frame([5,12]),'spx-atten-edge':build_spx_atten_frame([15,31])})
    for name,raw in sources.items():
        files[name+'-control.ec3']=raw*3
        for bsid in range(11,17):
            key=f'{name}-bsid{bsid}';packet=identify(raw,bsid)
            files[key+'.ec3']=packet*3;files[key+'.mkv']=movie([packet]*3)
            cases.append(dict(name=key,file=key+'.ec3',video=key+'.mkv',control=name+'-control.ec3',bsid=bsid))
    for bsid in [9,10,17,31]:
        key=f'invalid-bsid{bsid}';files[key+'.ec3']=identify(sources['long'],bsid)
        invalid.append(dict(file=key+'.ec3',bsid=bsid))
    files['manifest.json']=(json.dumps(dict(cases=cases,invalid=invalid),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
