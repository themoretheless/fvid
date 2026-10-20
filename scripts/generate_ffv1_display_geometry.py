#!/usr/bin/env python3
"""Authored display/crop fixtures using independent Python FFV1 packets only."""
import hashlib,json
from generate_ffv1_alpha import D,movie,packet,model

def sha(data):return hashlib.sha256(data).hexdigest()
def serialize(values,depth):return bytes(values) if depth==8 else b''.join(v.to_bytes(2,'little') for v in values)
def main():
    sources=[]
    for file in ['ffv1-alpha.json','ffv1-v3-alpha.json']:
        sources+=json.loads((D/file).read_text())['cases']
    # Independent ordinary v1 streams exercise the no-alpha path too.
    for depth in [8,16]:
        maximum=(1<<depth)-1;bias=1<<depth
        for layout in ['gray','420','444','rgb']:
            base=next(c for c in sources if c['depth']==depth and c['layout']==layout and c['version']==1 and c['coder']==1)
            name=f'ffv1-display-no-alpha-{layout}-{depth}';frames=[];packets=[];models=model(1)
            for index,f in enumerate(base['frames']):
                data=(D/f['color']).read_bytes();values=list(data) if depth==8 else [int.from_bytes(data[i:i+2],'little') for i in range(0,len(data),2)]
                rgb=layout=='rgb';chroma=layout!='gray';sub=(1,1) if layout=='420' else (0,0)
                if rgb:
                    planes=[[],[],[]]
                    for red,green,blue in zip(values[::3],values[1::3],values[2::3]):
                        cb=blue-green;cr=red-green
                        for plane,value in zip(planes,[green+((cb+cr)>>2),cb+bias,cr+bias]):plane.append(value)
                    sizes=[(4,2)]*3
                else:
                    cw,ch=(2,1) if layout=='420' else (4,2);n=cw*ch
                    planes=[values[:8]]+([values[8:8+n],values[8+n:8+2*n]] if chroma else [])
                    sizes=[(4,2)]+([(cw,ch)]*2 if chroma else [])
                p=packet(planes,sizes,depth,rgb,chroma,sub,1,1,index==0,models,False);file=f'{name}-{index}.packet';(D/file).write_bytes(p)
                packets.append(p);frames.append(dict(packet=file,color=f['color'],keyframe=index==0))
            data=movie(packets);file=name+'.mkv';(D/file).write_bytes(data)
            sources.append(dict(file=file,depth=depth,layout=layout,coder=1,version=1,size=[4,2],frames=frames,sha256=sha(data)))
    cases=[]
    for source in sources:
        packets=[(D/f['packet']).read_bytes() for f in source['frames']]
        keys=[f.get('keyframe',i==0) for i,f in enumerate(source['frames'])]
        size=source.get('size',[4,2]);private=(D/source['configuration']).read_bytes() if 'configuration' in source else b''
        for rotation in [0,90,180,270]:
            for crop in [[0,0,0,0],[1,1,0,0]]:
                name=source['file'][:-4]+f'-display-r{rotation}-crop{crop[0]}.mkv'
                data=movie(packets,roll=(90.0 if rotation==270 else -float(rotation)),crop=crop,size=size,private=private,keys=keys);(D/name).write_bytes(data)
                cases.append(dict(file=name,source=source,rotation=rotation,crop=crop,sha256=sha(data)))
    # All valid exponent pairs, including unnameable output layouts, are read
    # without refusing the codec; uncommon layouts promote to 4:4:4 losslessly.
    for depth in [8,16]:
        maximum=(1<<depth)-1
        for sx in range(5):
            for sy in range(5):
                cw=32>>sx;ch=32>>sy
                name=f'ffv1-display-chroma-{1<<sx}x{1<<sy}-{depth}';frames=[];packets=[];models=model(1)
                for index in range(2):
                    y=[(x*101+y*59+index*7)&maximum for y in range(32) for x in range(32)]
                    y[:4]=[0,maximum,1,maximum-1]
                    c=[[(i*97+p*43+index)&maximum for i in range(cw*ch)] for p in [1,2]]
                    alpha=[(x*43+y*83+index*11)&maximum for y in range(32) for x in range(32)]
                    alpha[:4]=[maximum,0,maximum-1,1]
                    planes=[y,*c,alpha];sizes=[(32,32),(cw,ch),(cw,ch),(32,32)]
                    p=packet(planes,sizes,depth,False,True,(sx,sy),1,1,index==0,models)
                    paths={}
                    for extension,data in [('packet',p),('color',serialize(y+c[0]+c[1],depth)),('alpha',serialize(alpha,depth))]:
                        file=f'{name}-{index}.{extension}';(D/file).write_bytes(data);paths[extension]=file
                    frames.append(paths);packets.append(p)
                file=name+'.mkv';data=movie(packets,size=(32,32));(D/file).write_bytes(data)
                source=dict(file=file,depth=depth,layout='custom',subsampling=[1<<sx,1<<sy],size=[32,32],frames=frames,version=1,coder=1)
                for rotation in [0,90,180,270]:
                    for crop in [[0,0,0,0],[1,1,0,0]]:
                        file=name+f'-r{rotation}-crop{crop[0]}.mkv'
                        data=movie(packets,size=(32,32),roll=(90.0 if rotation==270 else -float(rotation)),crop=crop)
                        (D/file).write_bytes(data)
                        cases.append(dict(file=file,source=source,rotation=rotation,crop=crop,sha256=sha(data)))
    base=next(c for c in sources if c['layout']=='rgb' and c['depth']==16 and c['version']==1)
    data=movie([(D/f['packet']).read_bytes() for f in base['frames']],roll=-90.,crop=[1,0,0,0],display=[6,2]);file='ffv1-display-alpha-nonsquare.mkv';(D/file).write_bytes(data)
    aspect=dict(file=file,source=base,rotation=90,crop=[1,0,0,0],sha256=sha(data))
    (D/'ffv1-display-geometry.json').write_text(json.dumps(dict(cases=cases,aspect=aspect,provenance='Synthetic authored FFV1 Range/custom Range/Rice packets with stored rotation/crop metadata. Full-depth sample arrays are reused only from authored fixtures; no private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
