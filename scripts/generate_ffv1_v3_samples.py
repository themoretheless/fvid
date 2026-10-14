#!/usr/bin/env python3
"""Independent v3 slice payloads, separate context sets, persistent sample models."""
import hashlib,json
from generate_ffv1_v3_configuration import config,sliced,element,uint,D,Range,TABLE
from generate_ffv1_rice_fixtures import Rice,coded_line
class CustomRange(Range):
    def __init__(self,table):super().__init__();self.table=table
    def bit(self,state,index,value):
        split=(self.range*state[index])>>8
        if value:self.low+=self.range-split;self.range=split;state[index]=self.table[state[index]]
        else:self.range-=split;state[index]=256-self.table[256-state[index]]
        if self.range<256:self.normalize()
def movie(private,packets,w,h,rgb):
    colour=element(0x55b0,uint(0x55b1,0 if rgb else 6)+uint(0x55b9,2))
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')+element(0x63a2,private)+uint(0x23e383,20000000)+element(0xe0,uint(0xb0,w)+uint(0xba,h)+colour)
    cluster=uint(0xe7,0)+b''.join(element(0xa3,b'\x81'+(i*20).to_bytes(2,'big')+(b'\x80' if i==0 else b'\0')+p) for i,p in enumerate(packets))
    return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,cluster))
def main():
    cases=[]
    for coder in [0,1,2]:
        for depth in [8,10,12,16]:
            for layout in ['gray','420','422','444','440','411','410','rgb']:
                for grid in [(1,1),(3,2)]:
                    w,h=(17,9) if grid==(1,1) else (18,12)
                    rgb=layout=='rgb';shift={'420':(1,1),'422':(1,0),'440':(0,1),'411':(2,0),'410':(2,2)}.get(layout,(0,0));sub=tuple(1<<s for s in shift)
                    ec=(depth+coder)%2;custom=[0]+[min(255,max(1,n+int(n<240))) for n in TABLE[1:]]
                    private,initial,_=config(coder=coder,depth=depth,rgb=rgb,chroma=layout!='gray',grid=grid,ec=ec,sx=shift[0],sy=shift[1],custom=custom if coder==2 else None)
                    table=custom if coder==2 else TABLE
                    name=f'ffv1-v3-{layout}-{depth}-coder{coder}-grid{grid[0]}x{grid[1]}'
                    (D/(name+'.config')).write_bytes(private)
                    states={};packets=[];frames=[];maximum=(1<<depth)-1
                    for frame in range(2):
                        source=[(i*53+frame*137)&maximum for i in range(w*h)]
                        if rgb:
                            pixels=[(v,(v*3+19)&maximum,maximum-v) for v in source]
                            full=[v for p in pixels for v in p]
                            output=bytes(full) if depth==8 else b''.join(v.to_bytes(2,'little') for v in full)
                        else:
                            cw,ch=(w+sub[0]-1)//sub[0],(h+sub[1]-1)//sub[1]
                            chroma=[[(i*71+p*101+frame*5)&maximum for i in range(cw*ch)] for p in range(2)] if layout!='gray' else [[1<<(depth-1)]*(w*h)]*2
                            full=source+chroma[0]+chroma[1]
                            output=bytes(full) if depth==8 else b''.join(v.to_bytes(2,'little') for v in full)
                        regions=[(x,y) for y in range(grid[1]) for x in range(grid[0])]
                        if frame:regions.reverse() # adaptation follows geometry, not packet order
                        packet=b''
                        for index,(x,y) in enumerate(regions):
                            left,right=x*w//grid[0],(x+1)*w//grid[0];top,bottom=y*h//grid[1],(y+1)*h//grid[1]
                            sw,sh=right-left,bottom-top
                            if rgb:
                                planes=[[],[],[]];bias=1<<depth
                                for yy in range(top,bottom):
                                    for xx in range(left,right):
                                        red,green,blue=pixels[yy*w+xx];base=blue if 9<=depth<=15 else green
                                        cb=(green if 9<=depth<=15 else blue)-base;cr=red-base
                                        for p,v in zip(planes,[base+((cb+cr)>>2),cb+bias,cr+bias]):p.append(v)
                                sizes=[(sw,sh)]*3
                            else:
                                planes=[[source[yy*w+xx] for yy in range(top,bottom) for xx in range(left,right)]]
                                sizes=[(sw,sh)]
                                if layout!='gray':
                                    pw,ph=(sw+sub[0]-1)//sub[0],(sh+sub[1]-1)//sub[1]
                                    for p in range(2):planes.append([chroma[p][(top//sub[1]+yy)*cw+left//sub[0]+xx] for yy in range(ph) for xx in range(pw)])
                                    sizes += [(pw,ph)]*2
                            r=CustomRange(table)
                            if index==0:r.bit([128],0,frame==0)
                            header=[128]*32
                            for value in [x,y,0,0,0,1,3,1,1]:r.integer(header,value)
                            key=(x,y)
                            if key not in states:
                                states[key]=Rice() if coder==0 else [[row.copy() for row in initial[0]],[row.copy() for row in initial[1]]]
                            entropy=states[key]
                            if coder==0:entropy.bits='';r.bit([129],0,False)
                            indexes=[0]*3
                            order=[(p,yy) for yy in range(sh) for p in range(3)] if rgb else [(p,yy) for p in range(len(planes)) for yy in range(sizes[p][1])]
                            for p,yy in order:
                                pw,_=sizes[p];bits=depth+int(rgb);group=int(p!=0)
                                differences,contexts=coded_line(planes[p],pw,yy,bits,p==0)
                                if coder==0:indexes[p]=entropy.line(differences,contexts,bits,group,indexes[p])
                                else:
                                    for diff,context in zip(differences,contexts):r.integer(entropy[group][abs(context)],diff,True)
                            payload=r.finish()+(entropy.finish() if coder==0 else b'')
                            packet+=sliced(payload,ec)
                        file=f'{name}-{frame}.packet';reference=f'{name}-{frame}.samples'
                        (D/file).write_bytes(packet);(D/reference).write_bytes(output);packets.append(packet)
                        frames.append(dict(packet=file,reference=reference,sha256=hashlib.sha256(packet).hexdigest(),reference_sha256=hashlib.sha256(output).hexdigest()))
                    data=movie(private,packets,w,h,rgb);file=name+'.mkv';(D/file).write_bytes(data)
                    cases.append(dict(file=file,configuration=name+'.config',layout=layout,coder=coder,depth=depth,size=[w,h],grid=grid,ec=ec,frames=frames,sha256=hashlib.sha256(data).hexdigest()))
    invalid=[];quantizer_changes=[]
    private,initial,_=config(chroma=False,grid=(3,2))
    (D/'ffv1-v3-invalid.config').write_bytes(private)
    regions=[(x,y,0,0) for y in range(2) for x in range(3)]
    def zeros(regions,key=True,q=(0,1),structure=3,states=None):
        if states is None:states={}
        packet=b''
        for i,(x,y,sw,sh) in enumerate(regions):
            r=Range()
            if i==0:r.bit([128],0,key)
            header=[128]*32
            for v in [x,y,sw,sh,*q,structure,1,1]:r.integer(header,v)
            region=(x,y,sw,sh)
            if region not in states:states[region]=initial[0][0].copy()
            model=states[region]
            for _ in range(36*(sw+1)*(sh+1)):r.integer(model,0,True)
            packet+=sliced(r.finish(),True)
        return packet
    good=zeros(regions);(D/'ffv1-v3-invalid-recovery.packet').write_bytes(good)
    qstates={};assert zeros(regions,states=qstates)==good
    qpacket=zeros(regions,False,q=(1,1),states=qstates)
    qfile='ffv1-v3-nonkey-quantizer-change.packet';qmovie='ffv1-v3-nonkey-quantizer-change.mkv'
    (D/qfile).write_bytes(qpacket);(D/qmovie).write_bytes(movie(private,[good,qpacket],18,12,False))
    qreference='ffv1-v3-nonkey-quantizer-change.samples';(D/qreference).write_bytes(bytes(216)+bytes([128])*432)
    quantizer_changes.append(dict(file=qfile,movie=qmovie,reference=qreference,sha256=hashlib.sha256(qpacket).hexdigest()))
    merged=[(0,0,1,0),(2,0,0,0),(0,1,2,0)];states={};merged_packets=[zeros(merged,states=states),zeros(list(reversed(merged)),False,states=states)]
    name='ffv1-v3-merged-slices';(D/(name+'.config')).write_bytes(private);frames=[]
    for i,p in enumerate(merged_packets):
        file=f'{name}-{i}.packet';reference=f'{name}-{i}.samples';samples=bytes(216)+bytes([128])*432
        (D/file).write_bytes(p);(D/reference).write_bytes(samples)
        frames.append(dict(packet=file,reference=reference,sha256=hashlib.sha256(p).hexdigest(),reference_sha256=hashlib.sha256(samples).hexdigest()))
    data=movie(private,merged_packets,18,12,False);file=name+'.mkv';(D/file).write_bytes(data)
    cases.append(dict(file=file,configuration=name+'.config',layout='gray',coder=1,depth=8,size=[18,12],grid=[3,2],ec=1,frames=frames,sha256=hashlib.sha256(data).hexdigest()))
    for label,packet,error in [
        ('overlap',zeros([regions[0],regions[0],*regions[2:]]),'overlapping FFV1 slices'),
        ('missing',zeros(regions[:-1]),'FFV1 frame has missing slices'),
        ('region',zeros([(3,0,0,0),*regions[1:]]),'FFV1 slice region exceeds raster'),
        ('quantizer',zeros(regions,q=(2,1)),'invalid FFV1 slice quantizer index'),
        ('structure',zeros(regions,structure=4),'invalid FFV1 picture structure'),
        ('changed-layout',zeros([(0,0,1,0),*regions[2:]],False),'FFV1 nonkeyframe changes slice layout'),
    ]:
        file=f'ffv1-v3-invalid-{label}.packet';(D/file).write_bytes(packet)
        movie_file=f'ffv1-v3-invalid-{label}.mkv';(D/movie_file).write_bytes(movie(private,[good,packet] if label=='changed-layout' else [packet],18,12,False))
        invalid.append(dict(file=file,movie=movie_file,error=error,sha256=hashlib.sha256(packet).hexdigest()))
    bad=bytearray(good);bad[0]^=1;file='ffv1-v3-invalid-frame-crc.packet';(D/file).write_bytes(bad)
    (D/'ffv1-v3-invalid-frame-crc.mkv').write_bytes(movie(private,[bad],18,12,False))
    invalid.append(dict(file=file,movie='ffv1-v3-invalid-frame-crc.mkv',error='FFV1 slice CRC mismatch',sha256=hashlib.sha256(bad).hexdigest()))
    for case in cases:case['configuration_sha256']=hashlib.sha256((D/case['configuration']).read_bytes()).hexdigest()
    for case in invalid+quantizer_changes:case['movie_sha256']=hashlib.sha256((D/case['movie']).read_bytes()).hexdigest()
    (D/'ffv1-v3-samples.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,quantizer_changes=quantizer_changes,invalid_configuration='ffv1-v3-invalid.config',recovery='ffv1-v3-invalid-recovery.packet',provenance='Original integer sample arrays and independent Python RFC 9043 range/Rice coders; custom transitions, two independent quantizer sets, coded initial states, odd and multislice rasters, reversed slice order in adapted nonkeyframe. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
