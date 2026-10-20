#!/usr/bin/env python3
"""Three independently coded RGB frames with presentation times reordered on disk."""
import hashlib,json
from pathlib import Path
from generate_ffv1_rgb_fixtures import packet
from generate_ffv1_gray_fixtures import element,uint
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ffv1-reordered-trim'
def generate():
    packets=[packet([(v, v+1, v+2)]*8,8)[0] for v in [20,60,100]]
    header=element(0x1a45dfa3,element(0x4282,b'matroska'))
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')+uint(0x23e383,20000000)+element(0xe0,uint(0xb0,4)+uint(0xba,2))
    prefix=element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))
    result={}
    for name,order in [('control.mkv',[0,1,2]),('reordered.mkv',[0,2,1])]:
        cluster=uint(0xe7,0)+b''.join(element(0xa3,b'\x81'+(i*20).to_bytes(2,'big')+b'\x80'+packets[i]) for i in order)
        result[name]=header+element(0x18538067,prefix+element(0x1f43b675,cluster))
    return result
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    files=generate()
    for name,data in files.items(): (OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Independent Python RFC 9043 range coding of three authored neutral RGB frames. No private media or external encoder.','qualification':'Deliberately nonconforming timestamp ordering for independent FFV1 keyframes (RFC 9559 section 10). Full export already accepts and sorts these frames. Regression qualifies consistent recovery during trimming, not format conformance.', 'trim_us':[0,30000],'expected_pts_ns':[0,20000000],'files':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n')
