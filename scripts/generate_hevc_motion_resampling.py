#!/usr/bin/env python3
"""Original monochrome reference videos and H.8.1.4.3 motion-grid oracles."""
import json,struct
from pathlib import Path
from generate_hevc_inter_layer_resampling import motion_oracle
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def main():
    cases=[]
    for label,ref,cur,depth,scaled,region in [('mono8',[64,48],[96,80],8,[0]*4,[0]*4),('mono16',[64,48],[128,96],16,[20,0,0,0],[-1,0,1,0]),('partial',[24,24],[40,40],10,[0]*4,[0]*4),('identity',[64,48],[64,48],16,[0]*4,[0]*4)]:
        c=dict(prefix='hevc-motion-resample-'+label,reference_size=ref,current_size=cur,reference_format=0,current_format=0,reference_depth=[depth,depth],current_depth=[depth,depth],scaled=scaled,region=region,phases=None)
        raw=f'YUV4MPEG2 W{ref[0]} H{ref[1]} F25:1 Ip A1:1 Cmono{depth if depth!=8 else ""}\n'.encode();frames=[]
        for f in (0,1):
            source,output,positions=motion_oracle(c,f)
            samples=[(x*17+y*31+f*47)%((1<<depth)) for y in range(ref[1]) for x in range(ref[0])]
            raw+=b'FRAME\n'+(bytes(samples) if depth==8 else struct.pack('<'+'H'*len(samples),*samples))
            frames.append(dict(samples=samples,input=source,output=output,positions=positions))
        c['frames']=frames;(D/(c['prefix']+'.y4m')).write_bytes(raw);cases.append(c)
    (D/'hevc-motion-resampling.json').write_text(json.dumps(dict(cases=cases,provenance='Original two-frame monochrome Y4M and rational motion-grid oracle. Motion operation acceptance only; no multilayer playback claim. No private source, external codec, FFmpeg or network.'),separators=(',',':'))+'\n')
    print('Monochrome motion cases:',len(cases),flush=True)
if __name__=='__main__':main()
