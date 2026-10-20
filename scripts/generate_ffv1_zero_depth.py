#!/usr/bin/env python3
"""Explicit compatibility fixtures: v1 bits_per_raw_sample=0 means 8 (RFC 9043)."""
import json,hashlib
from generate_ffv1_alpha import D,packet,movie,model

def main():
    cases=[]
    for coder in range(3):
        for layout in ['gray','420','444','rgb']:
            for alpha in [False,True]:
                name=f'ffv1-v1-zero-depth-{layout}-coder{coder}-alpha{int(alpha)}'
                frames=[];packets=[];models=model(coder)
                for i in range(2):
                    rgb=layout=='rgb';chroma=layout!='gray';sub=(1,1) if layout=='420' else (0,0)
                    if rgb:
                        color=[(n*37+i*19)&255 for n in range(24)]
                        planes=[[],[],[]]
                        for red,green,blue in zip(color[::3],color[1::3],color[2::3]):
                            cb=blue-green;cr=red-green
                            for p,v in zip(planes,[green+((cb+cr)>>2),cb+256,cr+256]):p.append(v)
                        sizes=[(4,2)]*3
                    else:
                        cw,ch=(2,1) if layout=='420' else (4,2)
                        planes=[[(n*31+i*17)&255 for n in range(8)]]
                        if chroma:planes += [[(n*43+p*61+i*23)&255 for n in range(cw*ch)] for p in [1,2]]
                        sizes=[(4,2)]+([(cw,ch)]*2 if chroma else [])
                        color=sum(planes,[])
                        if not chroma:color += [128]*16
                    opacity=[0,255,1,254,127,2,253,64+i]
                    if alpha:planes.append(opacity);sizes.append((4,2))
                    encoded=packet(planes,sizes,8,rgb,chroma,sub,1,coder,i==0,models,alpha,0)
                    paths={}
                    for ext,data in [('packet',encoded),('color',bytes(color))]+([('alpha',bytes(opacity))] if alpha else []):
                        file=f'{name}-{i}.{ext}';(D/file).write_bytes(data);paths[ext]=file
                    frames.append(paths);packets.append(encoded)
                file=name+'.mkv';data=movie(packets);(D/file).write_bytes(data)
                cases.append(dict(file=file,layout=layout,coder=coder,frames=frames,sha256=hashlib.sha256(data).hexdigest()))
    (D/'ffv1-zero-depth.json').write_text(json.dumps(dict(cases=cases,provenance='Authored Python packets, explicit zero depth header. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
