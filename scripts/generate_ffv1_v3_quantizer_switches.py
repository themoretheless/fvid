#!/usr/bin/env python3
"""Original seven-frame quantizer switches: retain, shrink, grow, keyframe reset."""
import hashlib,json
from generate_ffv1_v3_configuration import config,sliced,element,uint,D,TABLE
from generate_ffv1_v3_samples import CustomRange
from generate_ffv1_rice_fixtures import Rice,coded_line,signed
class Writer(CustomRange):
    def bit(self,state,index,value):
        if state[index]==0:
            assert not value,'zero-probability one cannot be encoded'
            return
        super().bit(state,index,value)
def contexts(p,w,y,bits,q,signed_samples=None):
    if signed_samples is None:signed_samples=bits==16
    def read(x,yy):
        if yy<0 or x < -1:return 0
        if x==-1:return 0 if yy==0 else read(0,yy-1)
        v=p[yy*w+min(x,w-1)]
        return signed(v,16) if signed_samples else v
    threshold=1 if q==0 else 2
    return [0 if q==1 or abs(signed(read(x-1,y)-read(x-1,y-1),8))<threshold else (1 if signed(read(x-1,y)-read(x-1,y-1),8)>0 else -1) for x in range(w)]
def movie(private,packets,keys,w,h):
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')+element(0x63a2,private)+uint(0x23e383,20000000)+element(0xe0,uint(0xb0,w)+uint(0xba,h))
    blocks=b''.join(element(0xa3,b'\x81'+(i*20).to_bytes(2,'big')+(b'\x80' if keys[i] else b'\0')+p) for i,p in enumerate(packets))
    return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,uint(0xe7,0)+blocks))
def main():
    cases=[]
    quantization=[[[0,126]]+[[127]]*4,[[127]]*5,[[1,125]]+[[127]]*4]
    sequence=[((0,1),True),((1,0),False),((1,0),True),((2,0),False),((0,2),False),((1,1),False),((1,1),True)]
    variants=[(coder,depth,rgb,False) for coder in [0,1,2] for depth in [8,10,12,16] for rgb in [False,True]]+[(1,8,False,True)]
    for coder,depth,rgb,zero_initial in variants:
        w,h,sw=12,6,6;maximum=(1<<depth)-1;bias=1<<depth
        table=[0]+[min(255,max(1,n+int(n<240))) for n in TABLE[1:]] if coder==2 else TABLE
        private,initial,_=config(coder=coder,depth=depth,rgb=rgb,chroma=rgb,grid=(2,1),count=3,quantization=quantization,initial_offset=15,custom=table if coder==2 else None,zero_state=zero_initial)
        name=f'ffv1-v3-switch-coder{coder}-depth{depth}-rgb{int(rgb)}'+('-initial0' if zero_initial else '');(D/(name+'.config')).write_bytes(private)
        histories={};active={};packets=[];frames=[]
        for index,(q,keyframe) in enumerate(sequence):
            output=[0]*(w*h*(3 if rgb else 1));packet=b''
            for position,sx in enumerate([0,1] if index%2==0 else [1,0]):
                if keyframe:
                    entropy=Rice() if coder==0 else [[row.copy() for row in initial[t]] for t in q]
                    if coder==0:
                        entropy.models=[[dict(count=1,total=4,drift=0,bias=0) for _ in initial[t]] for t in q]
                    histories[sx]=entropy
                else:
                    entropy=histories[sx]
                    for group in range(2):
                        if len(initial[q[group]])>len(initial[active[sx][group]]):
                            if coder==0:entropy.models[group]=[dict(count=1,total=4,drift=0,bias=0) for _ in initial[q[group]]]
                            else:entropy[group]=[[0]*32 for _ in initial[q[group]]]
                active[sx]=q
                zero=[coder!=0 and all(not any(row) for row in entropy[g]) for g in range(2)]
                if not rgb:
                    planes=[[(x+y+1) if index==4 or (coder!=0 and entropy[0][0][0]==0) else (x*13+y*37+index*19+sx*23)&maximum for y in range(h) for x in range(sw)]]
                elif zero[1]:
                    assert not zero[0]
                    planes=[[(bias-1)//2]*(sw*h)]+[[x+y+1 for y in range(h) for x in range(sw)] for _ in range(2)]
                elif zero[0] or index==4:
                    # At the equal-count switch, +/-1 gradients separate the two
                    # quantizers (threshold 1 versus 2), rather than merely naming
                    # a different but unobserved table in the slice header.
                    delta=1-(index&1)
                    planes=[[x+y+1 for y in range(h) for x in range(sw)],[bias+delta]*(sw*h),[bias-delta]*(sw*h)]
                else:
                    planes=[[],[],[]]
                    for y in range(h):
                        for x in range(sw):
                            red=(x*13+y*11+index*19+sx*23)&maximum;green=(x*7+y*29+index*17+sx*31)&maximum;blue=(x*41+y*3+index*5+sx*43)&maximum
                            base=blue if 9<=depth<=15 else green;cb=(green if 9<=depth<=15 else blue)-base;cr=red-base
                            for p,v in zip(planes,[base+((cb+cr)>>2),cb+bias,cr+bias]):p.append(v)
                for y in range(h):
                    for x in range(sw):
                        at=y*sw+x;destination=y*w+sx*sw+x
                        if rgb:
                            cb=planes[1][at]-bias;cr=planes[2][at]-bias;base=planes[0][at]-((cb+cr)>>2)
                            pixel=[cr+base,cb+base,base] if 9<=depth<=15 else [cr+base,base,cb+base]
                            assert all(0<=v<=maximum for v in pixel)
                            output[destination*3:destination*3+3]=pixel
                        else:output[destination]=planes[0][at]
                r=Writer(table)
                if position==0:r.bit([128],0,keyframe)
                header=[128]*32
                for v in [sx,0,0,0,*q,3,1,1]:r.integer(header,v)
                if coder==0:entropy.bits='';r.bit([129],0,False)
                run=[0]*3;bits=depth+int(rgb)
                order=[(p,y) for y in range(h) for p in range(3)] if rgb else [(0,y) for y in range(h)]
                for p,y in order:
                    group=int(p!=0);diffs,_=coded_line(planes[p],sw,y,bits,False);ctx=contexts(planes[p],sw,y,bits,q[group]);diffs=[-v if c<0 else v for v,c in zip(diffs,ctx)]
                    if coder==0:run[p]=entropy.line(diffs,ctx,bits,group,run[p])
                    else:
                        for v,c in zip(diffs,ctx):r.integer(entropy[group][abs(c)],v,True)
                packet+=sliced(r.finish()+(entropy.finish() if coder==0 else b''),True)
            if not rgb:output += [1<<(depth-1)]*(2*w*h)
            data=bytes(output) if depth==8 else b''.join(v.to_bytes(2,'little') for v in output)
            file=f'{name}-{index}.packet';reference=f'{name}-{index}.samples';(D/file).write_bytes(packet);(D/reference).write_bytes(data);packets.append(packet)
            frames.append(dict(packet=file,reference=reference,keyframe=keyframe,quantizers=q,sha256=hashlib.sha256(packet).hexdigest(),reference_sha256=hashlib.sha256(data).hexdigest()))
        data=movie(private,packets,[key for _,key in sequence],w,h);file=name+'.mkv';(D/file).write_bytes(data)
        cases.append(dict(file=file,configuration=name+'.config',configuration_sha256=hashlib.sha256(private).hexdigest(),depth=depth,rgb=rgb,coder=coder,zero_initial=zero_initial,size=[w,h],frames=frames,sha256=hashlib.sha256(data).hexdigest()))
    (D/'ffv1-v3-quantizer-switches.json').write_text(json.dumps(dict(cases=cases,provenance='Original seven-frame sample/RCT arrays and independent Python entropy coding. Quantizers change context count 2->1->2 and shape at equal count; shrinking/equal banks retain adaptation, growing banks are recreated (zero Range probabilities, default Rice models), keyframes restore configured initial states. No external codec, private media or network.'),indent=2)+'\n')
if __name__=='__main__':main()
