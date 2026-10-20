"""Independent PAFF P/B component-precision clips with scalar sample oracles."""
import json,hashlib
from pathlib import Path
from generate_avc_field_component_regressions import configuration,field,Writer
from generate_avc_component_residual import increment
from generate_avc_component_fractional import interpolate
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-field-inter-components'
def inter(bottom,second,mode,frame=1):
    bipred=mode=='b-bi';b=Writer();b.ue(0);b.ue(1 if bipred else 0);b.ue(0);b.u(2 if bipred else frame,4);b.u(1);b.u(int(bottom));b.u((2 if bipred else frame*4)+int(second),4)
    if bipred:b.u(1)
    b.u(0);b.u(0)
    if bipred:b.u(0)
    else:
        if mode=="p-weighted":
            b.ue(0);b.ue(0);b.u(1);b.se(1);b.se(2);b.u(1);b.se(1);b.se(3);b.se(1);b.se(-2)
        b.u(0)
    b.se(0);b.ue(0 if mode=="p-filtered" else 1)
    if mode=="p-filtered":b.se(0);b.se(0)
    if mode=='p-skip':b.ue(2)
    else:
        for address in range(2):
            b.ue(0);b.ue(3 if bipred else 0)
            vector=[1,0] if mode=="p-x" else [0,-3] if mode=="p-y" else [0,0]
            for value in (vector if address==0 else [0,0])*(2 if bipred else 1):b.se(value)
            b.ue(19 if mode=='p-residual' else 0)
            if mode=='p-residual':
                b.se(0)
                for block in range(16):b.u(1,2);b.u(block%2);b.u(1)
                for component in range(2):b.u(1);b.u(component);b.u(1)
    nal=b.nal(0x01 if bipred else 0x41)
    return len(nal).to_bytes(4,'big')+nal

def woven(fields):
    result=[]
    for p in range(3):
        width=32 if p==0 else 16;rows=16 if p==0 else 8;values=[]
        for row in range(rows):
            for bottom in [False,True]:values+=fields[bottom][p][row*width:(row+1)*width]
        result.append(values)
    return result

def generate():
    files={};cases=[]
    pairs=[(8,8),(10,10),(12,12),(14,14),(8,10),(10,8),(8,14),(14,8),(12,14),(14,12)]
    for y,c in pairs:
        for reverse in [False,True]:
            for mode in ['p-skip','p-zero','p-residual','b-bi','p-weighted','p-x','p-y','p-filtered']:
                name=f'y{y}-c{c}-'+('bottom' if reverse else 'top')+'-'+mode;config=configuration(y,c,weighted=mode=="p-weighted");order=[True,False] if reverse else [False,True];packets=[];first={};anchor={}
                for second,bottom in enumerate(order):
                    packet,planes=field(y,c,bottom,0,second==0,bool(second),'dc-filtered' if mode=='p-filtered' else 'pcm');packets.append(packet);first[bottom]=planes
                if mode=='b-bi':
                    for second,bottom in enumerate(order):
                        packet,planes=field(y,c,bottom,1,False,bool(second),'pcm');packets.append(packet);anchor[bottom]=planes
                    expected=[woven(first),[[ (a+b+1)//2 for a,b in zip(ap,bp)] for ap,bp in zip(woven(first),woven(anchor))],woven(anchor)]
                elif mode=='p-residual':
                    changed={}
                    for bottom in order:
                        changed[bottom]=[]
                        for p,depth in enumerate([y,c,c]):
                            width=32 if p==0 else 16;values=[]
                            for i,value in enumerate(first[bottom][p]):
                                sign=(1 if (i%width)//4%2==0 else -1) if p==0 else (1 if p==1 else -1)
                                values.append(max(0,min((1<<depth)-1,value+increment(26,depth,sign,p!=0))))
                            changed[bottom].append(values)
                    expected=[woven(first),woven(changed),woven(changed)]
                elif mode in ["p-weighted","p-x","p-y"]:
                    predicted={}
                    for bottom in order:
                        predicted[bottom]=[]
                        for p,depth in enumerate([y,c,c]):
                            source=first[bottom][p]
                            if mode=="p-weighted":values=[max(0,min((1<<depth)-1,v+[2,3,-2][p]*(1<<(depth-8)))) for v in source]
                            else:values=interpolate(source,32 if p==0 else 16,depth,1 if mode=="p-x" else -3,0 if mode=="p-x" else 1,p!=0)
                            predicted[bottom].append(values)
                    expected=[woven(first),woven(predicted)]
                else:expected=[woven(first),woven(first)]
                for second,bottom in enumerate(order):packets.append(inter(bottom,bool(second),mode))
                if mode=='p-residual':
                    for second,bottom in enumerate(order):packets.append(inter(bottom,bool(second),'p-skip',frame=2))
                frames=[]
                decode_order=[0,2,1] if mode=='b-bi' else list(range(len(expected)))
                for index,planes in enumerate(expected):
                    entry={}
                    for p,key in enumerate(['y','cb','cr']):filename=f'{name}-{index}.{key}';files[filename]=b''.join(v.to_bytes(2,'little') for v in planes[p]);entry[key]=filename
                    frames.append(entry)
                packet_names=[];coded=[]
                for index,packet in enumerate(packets):
                    filename=f'{name}-{index}.packet';files[filename]=packet;packet_names.append(filename)
                    coded.append((decode_order[index//2]*2+index%2,index==0,packet))
                files[name+'.mp4']=mux(config,coded,32,32,100);files[name+'.avcc']=config
                cases.append({'file':name+'.mp4','configuration':name+'.avcc','packets':packet_names,'luma_depth':y,'chroma_depth':c,'bottom_first':reverse,'mode':mode,'decode_order':decode_order,'frames':frames})
    files['manifest.json']=(json.dumps({'provenance':'160 authored40/60ms PAFF Pskip/zeroP/signed residual+echo/reordered Bi/explicit weights/axial fractional motion/filtered prediction clips. Independent component DC scaling and woven samples. No private media/encoder/network.','cases':cases,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
