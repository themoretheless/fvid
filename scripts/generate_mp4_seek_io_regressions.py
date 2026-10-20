#!/usr/bin/env python3
"""Three authored, independently decodable colored VP9 samples in one MP4 chunk."""
import hashlib,json
from pathlib import Path
from generate_vp9_chroma import coloured,mp4,table
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-seek-io'
def generate():
    packets=[];files={}
    for index,q in enumerate([0,64,128]):
        packet,oracle=coloured(8,'420',side=16,base_q=q,dc_quant=table('DC_QLOOKUP')[0][q])
        packets.append(packet);files[f'frame-{index}.packet']=packet;files[f'frame-{index}.yuv16']=bytes(oracle)
    assert len(set(map(len,packets)))==1
    assert len(set(packets))==3
    files['three-colors.mp4']=mp4(packets,8,'420',size=(16,16),sync_samples=[1,2,3])
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True);files=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored VP9 arithmetic-coded colored keyframes, quantizer tables from owned code and analytic sample oracles. No private media, encoder or network.','failure':'A test Seek adapter moves to sample 2 and returns an I/O error. After reading sample 0, recovery reads sample 1 and must not reuse its stale cached position.','timescale':50,'pts':[0,1,2],'files':{name:hashlib.sha256(data).hexdigest() for name,data in files.items()}},indent=2)+'\n')
