#!/usr/bin/env python3
"""Authored color/alpha arrays, independent legacy FFV1 coders, no external codecs."""
import hashlib,json
from generate_ffv1_rgb_fixtures import Range,TABLE,element,uint,D
from generate_ffv1_v3_samples import CustomRange
from generate_ffv1_rice_fixtures import Rice,coded_line
def packet(planes,sizes,depth,rgb,chroma,sub,version,coder,key,models):
    r=CustomRange(TABLE);r.bit([128],0,key)
    custom=[0]+[min(255,max(1,n+int(n<240))) for n in TABLE[1:]]
    if key:
        h=[128]*32;r.integer(h,version);r.integer(h,coder)
        if coder==2:
            for i in range(1,256):r.integer(h,custom[i]-TABLE[i],True)
        r.integer(h,int(rgb))
        if version:r.integer(h,depth)
        r.bit(h,0,chroma);r.integer(h,sub[0]);r.integer(h,sub[1]);r.bit(h,0,True)
        for _ in range(5):r.integer([128]*32,127)
    r.table=custom if coder==2 else TABLE
    if coder==0:models.bits='';r.bit([129],0,False)
    order=[(p,y) for y in range(2) for p in range(4)] if rgb else [(p,y) for p in range(len(planes)) for y in range(sizes[p][1])]
    run=[0]*4;bits=depth+int(rgb)
    for p,y in order:
        alpha=p==len(planes)-1;group=2 if alpha else int(p!=0)
        w,_=sizes[p];diffs,ctx=coded_line(planes[p],w,y,bits,False)
        if coder==0:run[p]=models.line(diffs,ctx,bits,group,run[p])
        else:
            for value in diffs:r.integer(models[group],value,True)
    return r.finish()+(models.finish() if coder==0 else b'')
def model(coder):
    if coder:
        return [[128]*32 for _ in range(3)]
    m=Rice();m.models=[[dict(count=1,total=4,drift=0,bias=0)] for _ in range(3)];return m
def movie(packets):
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')+uint(0x23e383,20000000)+element(0xe0,uint(0xb0,4)+uint(0xba,2))
    blocks=b''.join(element(0xa3,b'\x81'+(i*20).to_bytes(2,'big')+(b'\x80' if i==0 else b'\0')+p) for i,p in enumerate(packets))
    return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,uint(0xe7,0)+blocks))
def main():
    cases=[]
    for depth in [8,10,12,16]:
        maximum=(1<<depth)-1;bias=1<<depth
        for layout in ['gray','420','444','rgb']:
            for coder in [0,1,2]:
                for version in ([0,1] if depth==8 else [1]):
                    rgb=layout=='rgb';chroma=layout!='gray';sub=(1,1) if layout=='420' else (0,0)
                    name=f'ffv1-alpha-{layout}-{depth}-coder{coder}-v{version}';models=model(coder);frames=[];packets=[]
                    for frame in range(2):
                        alpha=[0,maximum,1,maximum-1,maximum//2,2+frame,maximum//3,0 if frame else maximum]
                        if rgb:
                            pixels=[(0,0,0),(maximum,0,0),(0,maximum,0),(0,0,maximum),(maximum,maximum,maximum),(1,2,3),(maximum,1,maximum-1),(123+frame,77+frame,31+frame)]
                            planes=[[],[],[]]
                            for red,green,blue in pixels:
                                cb=blue-green;cr=red-green
                                for p,v in zip(planes,[green+((cb+cr)>>2),cb+bias,cr+bias]):p.append(v)
                            sizes=[(4,2)]*3;color=[v for p in pixels for v in p]
                        else:
                            y=[0,maximum,1,maximum-1,maximum//2,2+frame,maximum//3,3+frame]
                            cw,ch=(2,1) if layout=='420' else (4,2)
                            planes=[y]+[[(i*97+p*43+frame)&maximum for i in range(cw*ch)] for p in range(2)] if chroma else [y]
                            sizes=[(4,2)]+[(cw,ch)]*2 if chroma else [(4,2)]
                            color=[v for p in planes for v in p]
                            if not chroma:color += [1<<(depth-1)]*16
                        planes.append(alpha);sizes.append((4,2))
                        encoded=packet(planes,sizes,depth,rgb,chroma,sub,version,coder,frame==0,models);keypacket=packet(planes,sizes,depth,rgb,chroma,sub,version,coder,True,model(coder))
                        def store(extension,data):
                            file=f'{name}-{frame}.{extension}';(D/file).write_bytes(data);return file
                        serialize=lambda values:bytes(values) if depth==8 else b''.join(v.to_bytes(2,'little') for v in values)
                        frames.append(dict(packet=store('packet',encoded),keypacket=store('keypacket',keypacket),color=store('color',serialize(color)),alpha=store('alpha',serialize(alpha)),sha256=hashlib.sha256(encoded).hexdigest(),keypacket_sha256=hashlib.sha256(keypacket).hexdigest(),color_sha256=hashlib.sha256(serialize(color)).hexdigest(),alpha_sha256=hashlib.sha256(serialize(alpha)).hexdigest()))
                        packets.append(encoded)
                    file=name+'.mkv';data=movie(packets);(D/file).write_bytes(data);cases.append(dict(file=file,depth=depth,layout=layout,coder=coder,version=version,frames=frames,sha256=hashlib.sha256(data).hexdigest()))
    planes=[[0]*8,[256]*8,[256]*8,[256]+[0]*7]
    bad=packet(planes,[(4,2)]*4,8,True,True,(0,0),1,1,True,model(1));bad_file='ffv1-alpha-invalid-rgb-range.packet';(D/bad_file).write_bytes(bad);(D/'ffv1-alpha-invalid-rgb-range.mkv').write_bytes(movie([bad]))
    (D/'ffv1-alpha.json').write_text(json.dumps(dict(cases=cases,invalid=[dict(file=bad_file,error='FFV1 alpha reconstruction exceeds sample depth')],provenance='Authored RGBA/YUVA/gray-alpha arrays and independent Python range/Rice coders. Alpha has independent context state and full coded precision. RGB with alpha uses the ordinary Green-base RCT even at 10/12 bits. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
