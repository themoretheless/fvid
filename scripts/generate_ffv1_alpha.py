#!/usr/bin/env python3
"""Authored color/alpha arrays, independent legacy FFV1 coders, no external codecs."""
import hashlib,json,struct
from generate_ffv1_rgb_fixtures import Range,TABLE,element,uint,D
from generate_ffv1_v3_samples import CustomRange
from generate_ffv1_rice_fixtures import Rice,coded_line
def packet(planes,sizes,depth,rgb,chroma,sub,version,coder,key,models,has_alpha=True,header_depth=None):
    r=CustomRange(TABLE);r.bit([128],0,key)
    custom=[0]+[min(255,max(1,n+int(n<240))) for n in TABLE[1:]]
    if key:
        h=[128]*32;r.integer(h,version);r.integer(h,coder)
        if coder==2:
            for i in range(1,256):r.integer(h,custom[i]-TABLE[i],True)
        r.integer(h,int(rgb))
        if version:r.integer(h,depth if header_depth is None else header_depth)
        r.bit(h,0,chroma);r.integer(h,sub[0]);r.integer(h,sub[1]);r.bit(h,0,has_alpha)
        for _ in range(5):r.integer([128]*32,127)
    r.table=custom if coder==2 else TABLE
    if coder==0:models.bits='';r.bit([129],0,False)
    order=[(p,y) for y in range(2) for p in range(len(planes))] if rgb else [(p,y) for p in range(len(planes)) for y in range(sizes[p][1])]
    run=[0]*4;bits=depth+int(rgb)
    for p,y in order:
        alpha=has_alpha and p==len(planes)-1;group=2 if alpha else int(p!=0)
        w,_=sizes[p];diffs,ctx=coded_line(planes[p],w,y,bits,False,signed_samples=not rgb and depth==16)
        if coder==0:run[p]=models.line(diffs,ctx,bits,group,run[p])
        else:
            for value in diffs:r.integer(models[group],value,True)
    return r.finish()+(models.finish() if coder==0 else b'')
def model(coder):
    if coder:
        return [[128]*32 for _ in range(3)]
    m=Rice();m.models=[[dict(count=1,total=4,drift=0,bias=0)] for _ in range(3)];return m
def movie(packets,roll=None,crop=None,display=None,size=(4,2),private=b'',keys=None):
    video=uint(0xb0,size[0])+uint(0xba,size[1])
    if crop is not None:
        video+=b''.join(uint(tag,value) for tag,value in zip([0x54cc,0x54bb,0x54dd,0x54aa],crop))
    if roll is not None:
        video+=element(0x7670,uint(0x7671,0)+element(0x7675,struct.pack('>d',roll)))
    if display is not None:
        video+=uint(0x54b0,display[0])+uint(0x54ba,display[1])
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')+uint(0x23e383,20000000)+(element(0x63a2,private) if private else b'')+element(0xe0,video)
    blocks=b''.join(element(0xa3,b'\x81'+(i*20).to_bytes(2,'big')+(b'\x80' if (keys[i] if keys is not None else i==0) else b'\0')+p) for i,p in enumerate(packets))
    return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,uint(0xe7,0)+blocks))
def main():
    cases=[]
    for depth in range(8,17):
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
    opaque=[]
    for depth in range(8,17):
        maximum=(1<<depth)-1;bias=1<<depth
        for layout in ['gray','420','444','rgb']:
            rgb=layout=='rgb';chroma=layout!='gray';sub=(1,1) if layout=='420' else (0,0)
            cw,ch=(2,1) if layout=='420' else (4,2)
            planes=([[maximum//2]*8,[bias]*8,[bias]*8] if rgb else
                [[maximum//2]*8]+([[1<<(depth-1)]*(cw*ch)]*2 if chroma else []))
            sizes=[(4,2)]*3 if rgb else [(4,2)]+([(cw,ch)]*2 if chroma else [])
            planes.append([maximum]*8);sizes.append((4,2))
            encoded=packet(planes,sizes,depth,rgb,chroma,sub,1,1,True,model(1))
            name=f'ffv1-alpha-opaque-rotate-{layout}-{depth}'
            data=movie([encoded]);(D/(name+'.mkv')).write_bytes(data)
            (D/(name+'.packet')).write_bytes(encoded)
            opaque.append(dict(file=name+'.mkv',packet=name+'.packet',depth=depth,layout=layout,
                sha256=hashlib.sha256(data).hexdigest(),packet_sha256=hashlib.sha256(encoded).hexdigest()))
    source=next(c for c in cases if c['layout']=='rgb' and c['depth']==16 and c['coder']==1)
    packets=[(D/f['packet']).read_bytes() for f in source['frames']]
    data=movie(packets,roll=-90.0,crop=(2,0,0,0));file='ffv1-alpha-display-rgb16-clock-crop.mkv';(D/file).write_bytes(data)
    display=[dict(file=file,source=source['file'],sha256=hashlib.sha256(data).hexdigest(),rotation=90,source_crop=[2,0,0,0],display_crop=[0,2,0,0])]
    source=next(c for c in cases if c['layout']=='420' and c['depth']==16 and c['coder']==1)
    data=movie([(D/f['packet']).read_bytes() for f in source['frames']]);file='ffv1-alpha-scale-phase-420-16.mkv';(D/file).write_bytes(data)
    scale_phase=dict(file=file,source=source['file'],sha256=hashlib.sha256(data).hexdigest(),source_indices=[1,3,5,7],wrong_indices=[0,2,4,6])
    planes=[[0]*8,[256]*8,[256]*8,[256]+[0]*7]
    bad=packet(planes,[(4,2)]*4,8,True,True,(0,0),1,1,True,model(1));bad_file='ffv1-alpha-invalid-rgb-range.packet';(D/bad_file).write_bytes(bad);(D/'ffv1-alpha-invalid-rgb-range.mkv').write_bytes(movie([bad]))
    (D/'ffv1-alpha.json').write_text(json.dumps(dict(cases=cases,opaque_rotations=opaque,display_geometry=display,scale_phase=scale_phase,invalid=[dict(file=bad_file,error='FFV1 alpha reconstruction exceeds sample depth')],provenance='Authored RGBA/YUVA/gray-alpha arrays and independent Python range/Rice coders. Alpha has independent context state and full coded precision. RGB with alpha uses the ordinary Green-base RCT even at 10/12 bits. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
