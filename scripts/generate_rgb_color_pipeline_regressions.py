#!/usr/bin/env python3
"""Authored legacy FFV1 color/alpha clips; no private media or external codecs."""
import hashlib,json
from pathlib import Path
from generate_ffv1_rgb_fixtures import packet as rgb_packet,movie as rgb_movie,element,uint
from generate_ffv1_alpha import packet as alpha_packet,movie as alpha_movie,model
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/rgb-color-pipeline'

def serialize(values,depth):
    return bytes(values) if depth==8 else b''.join(v.to_bytes(2,'little') for v in values)

def header(data):
    old=element(0x1a45dfa3,element(0x4282,b'matroska'));assert data.startswith(old)
    new=element(0x1a45dfa3,uint(0x4286,1)+uint(0x42f7,1)+uint(0x42f2,4)+uint(0x42f3,8)+element(0x4282,b'matroska')+uint(0x4287,4)+uint(0x4285,4))
    return new+data[len(old):]

def generate():
    files={};cases=[]
    configurations=[('rgb',depth,False) for depth in [8,10,12,16]]+[('rgb',depth,True) for depth in [8,16]]+[('yuv444',depth,True) for depth in [8,10,12,16]]
    for layout,depth,has_alpha,smooth in [(*c,smooth) for c in configurations for smooth in [False,True]]:
        maximum=(1<<depth)-1;bias=1<<depth;name=f'{layout}-{depth}'+('-alpha' if has_alpha else '')+('-smooth' if smooth else '')
        packets=[];frames=[]
        for index in range(3):
            opacity=[0,maximum,1,maximum-1,maximum//2,2+index,maximum//3,maximum if index%2==0 else 0]
            pixels=[(0,0,0),(maximum,0,0),(0,maximum,0),(0,0,maximum),(maximum,maximum,maximum),(1,2,3),(maximum,1,maximum-1),(123+index,77+index,31+index)]
            if smooth:
                opacity=[maximum//4+(i%3)*maximum//12+index for i in range(8)]
                pixels=[tuple(maximum//3+(i%3)*maximum//18+c*maximum//24+index for c in range(3)) for i in range(8)]
            if layout=='rgb':
                color=[v for pixel in pixels for v in pixel]
                if not has_alpha:
                    encoded,_=rgb_packet(pixels,depth)
                else:
                    planes=[[],[],[]]
                    for red,green,blue in pixels:
                        cb=blue-green;cr=red-green
                        for p,v in zip(planes,[green+((cb+cr)>>2),cb+bias,cr+bias]):p.append(v)
                    encoded=alpha_packet(planes+[opacity],[(4,2)]*4,depth,True,True,(0,0),1,1,True,model(1))
            else:
                y=[0,maximum,1,maximum-1,maximum//2,2+index,maximum//3,3+index]
                if smooth:y=[maximum//3+(i%3)*maximum//18+index for i in range(8)]
                planes=[y]+[[(i*97+p*43+index)&maximum for i in range(8)] for p in range(2)]
                color=[v for plane in planes for v in plane]
                encoded=alpha_packet(planes+[opacity],[(4,2)]*4,depth,False,True,(0,0),1,1,True,model(1))
            packets.append(encoded);color_file=f'{name}-{index}.color';files[color_file]=serialize(color,depth)
            record={'color':color_file}
            if has_alpha:
                alpha_file=f'{name}-{index}.alpha';files[alpha_file]=serialize(opacity,depth);record['alpha']=alpha_file
            frames.append(record)
        video=rgb_movie(packets,depth) if not has_alpha else alpha_movie(packets,keys=[True]*3)
        file=name+'.mkv';files[file]=header(video);cases.append({'file':file,'layout':layout,'depth':depth,'alpha':has_alpha,'frames':frames})
    return files,cases

def main():
    OUT.mkdir(parents=True,exist_ok=True);files,cases=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored eight-pixel RGB or YUV444 arrays and varying opacity, independently range-coded as three FFV1 v1 keyframes. 60 ms clips; no private media, codec parameters, external encoder or network.','cases':cases,'files':{name:hashlib.sha256(data).hexdigest() for name,data in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
