"""Owned texture IV/DI B-list, POC-remapping and weighted-pixel oracles."""
import json,struct
from functools import lru_cache
import generate_hevc_texture_iv as p
from generate_hevc_cqt import banks,context,merge_index_bins,ue,se,OUT
from generate_hevc_colour_plane_fixtures import packed,escape
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_reconstruction import parameter_ids
from hevc_fixture_mp4 import mux
WEIGHTS=[[(1,2),(2,-3),(-1,7)],[(3,-1),(1,4),(0,9)]]

@lru_cache(None)
def parameters(enabled,weighted):
    movie=p.movie(enabled,False,0)
    config=movie[movie.index(b'hvcC')+4:];at=23;units=[]
    for _ in range(config[22]):
        count=int.from_bytes(config[at+1:at+3],'big');at+=3
        for _ in range(count):
            length=int.from_bytes(config[at:at+2],'big');at+=2
            units.append(config[at:at+length]);at+=length
    if weighted:
        units=[n for n in units if not((int.from_bytes(n[:2],'big')>>9)&63==34 and(int.from_bytes(n[:2],'big')>>3)&63==5)]
        units.append(parameter_ids(header_pps(5,0,True),0,[2,2]))
    return units

def pcm(bias=0):
    encoder,c=banks(0);encoder.decision(c['split0'],False)
    prefix=encoder.finish()
    pixels=bytes((x*3+y*5+17+bias)%256 for y in range(32)for x in range(32))+bytes([128])*512
    encoder.low=0;encoder.range=510;encoder.width=9
    return prefix+pixels+encoder.finish()

def header(layer,poc,data,stype,negative=(),positive=(),weighted=False,iv=False,maximum=5):
    value='1'+('0'if poc==0 else'')+ue({0:0,3:1,5:2}[layer])+ue(stype)+'1'
    if layer or poc:value+=f'{poc:08b}'
    if poc:
        value+='0'+ue(len(negative))+ue(len(positive))
        for distances in [negative,positive]:
            previous=0
            for distance in distances:value+=ue(distance-previous-1)+'1';previous=distance
        value+=ue(0)+ue(0)+'0' # no long-term syntax, temporal MVP disabled
    if layer:value+='101'if layer==5 and stype==0 else'0'
    value+='00'
    if stype!=2:
        value+='1'+ue(2)+ue(2)if layer==5 else'0'
        value+='00' # list modification flags, two/three current pictures
        value+='0'+'0' # mvd_l1_zero, cabac_init
        if weighted:
            value+=ue(1)+se(0) # luma/chroma denominator1
            for entries in WEIGHTS:
                value+='111'+'000' # luma weights present, chroma default
                for weight,offset in entries:value+=se(weight-2)+se(offset)
        elif layer:value+='0' # IC off, absent when weighted prediction applies
        value+=ue(5+int(iv)-maximum)
    value+=se(0)+ue(1)+'00000000'+'1';value+='0'*((-len(value))%8)
    return (((19 if poc==0 else 1)<<9)|(layer<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

def source(mode):
    encoder,c=banks(2);greater=context(encoder,198)
    encoder.decision(c['split0'],False);encoder.decision(c['skip0'],False)
    encoder.decision(c['pred'],False);encoder.decision(c['part0'],True);encoder.decision(c['merge'],False)
    encoder.decision(c['ip0'],mode=='bi')
    if mode!='bi':encoder.decision(c['ip4'],mode=='l1')
    for list,mv in [(0,4),(1,-8)]:
        if mode=='l0'and list==1 or mode=='l1'and list==0:continue
        encoder.decision(c['mvd'],False);encoder.decision(c['mvd'],True);encoder.decision(greater,True)
        remaining=abs(mv)-2;k=1
        while remaining>=1<<k:encoder.bypass(True);remaining-=1<<k;k+=1
        encoder.bypass(False)
        for shift in reversed(range(k)):encoder.bypass((remaining>>shift)&1)
        encoder.bypass(mv<0);encoder.decision(c['mvp'],False)
    encoder.decision(c['root'],False)
    return encoder.finish()

def target(index,maximum=5):
    encoder,c=banks(2);encoder.decision(c['split0'],False);encoder.decision(c['skip0'],True)
    merge_index_bins(encoder,c,index,maximum)
    return encoder.finish()

def movie(enabled,intra,mode,index,weighted,maximum=5):
    past=pcm();future=pcm(97)
    units=[[header(layer,0,past,2)for layer in [0,3,5]],
        [header(layer,2,future,2,negative=[2])for layer in [0,3,5]],
        [header(0,1,past,2,negative=[1],positive=[1]),
         header(3,1,past if intra else source(mode),2 if intra else 0,negative=[1],positive=[1]),
         header(5,1,target(index,maximum),0,negative=[1],positive=[1],weighted=weighted,iv=enabled,maximum=maximum)]]
    stream=b''.join(b'\0\0\0\1'+n for n in parameters(enabled,weighted)+[n for au in units for n in au])
    return mux(stream,8,32,32,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])

def oracle(enabled,intra,mode,index,weighted):
    l0=['past','current','future'];l1=['future','past','current']
    if enabled and not intra and index==0:
        refs,motion=([0,0],[1,-2])if mode=='bi'else(([0,1],[1,1])if mode=='l0'else([2,0],[-2,-2]))
        label='IV'
    elif enabled and index==(0 if intra else 1):refs,motion,label=[1,2],[0,0],'DI'
    elif enabled and intra and index==1:refs,motion,label=[1,2],[0,0],'DIShift'
    else:
        slot=index-2 if enabled else index
        refs=[slot if slot<3 else 0]*2;motion=[0,0];label='initial'
    source_pixels=[];target_pixels=[]
    for y in range(32):
        for x in range(32):
            def pattern(label,xx,yy):return(min(31,max(0,xx))*3+min(31,max(0,yy))*5+17+(97 if label=='future'else 0))%256
            def current(xx,yy):
                xx=min(31,max(0,xx));yy=min(31,max(0,yy))
                if intra:return pattern('past',xx,yy)
                a=pattern('past',xx,yy+1);b=pattern('future',xx,yy-2)
                return(a+b+1)//2 if mode=='bi'else a if mode=='l0'else b
            def sample(label,list):
                xx=x+(1 if label=='current'and index==1 and intra and enabled else 0)
                yy=y+motion[list]
                return current(xx,yy)if label=='current'else pattern(label,xx,yy)
            source_pixels.append(current(x,y))
            a=sample(l0[refs[0]],0);b=sample(l1[refs[1]],1)
            if weighted:
                w0,o0=WEIGHTS[0][refs[0]];w1,o1=WEIGHTS[1][refs[1]]
                value=(w0*a+w1*b+2*(o0+o1+1))//4
            else:value=(a+b+1)//2
            target_pixels.append(min(255,max(0,value)))
    horizontal=4 if label=='DIShift'else 0
    return label,refs,[[horizontal,motion[0]*4],[horizontal,motion[1]*4]],source_pixels,target_pixels

def generate():
    files={};cases=[];golden=bytearray()
    for enabled in [False,True]:
        for weighted in [False,True]:
            for intra,mode in [(False,'bi'),(False,'l0'),(False,'l1'),(True,'bi')]:
                for index in range(6 if enabled else 5):
                    maximum=6 if index==5 else 5
                    name='texture-iv-b-'+('on'if enabled else'off')+('-intra'if intra else'-'+mode)+('-weighted'if weighted else'')+('-merge6'if maximum==6 else'')+f'-{index}.mp4'
                    files[name]=movie(enabled,intra,mode,index,weighted,maximum)
                    label,refs,motion,source_pixels,target_pixels=oracle(enabled,intra,mode,index,weighted)
                    offsets=[]
                    for values in [source_pixels,target_pixels]:offsets.append(len(golden)//2);golden.extend(struct.pack('<1024H',*values))
                    cases.append(dict(file=name,enabled=enabled,weighted=weighted,intra=intra,mode=mode,index=index,maximum=maximum,label=label,references=refs,motion=motion,offsets=offsets))
    files['texture-iv-b-oracles.y16']=bytes(golden)
    files['texture-iv-b-manifest.json']=(json.dumps(dict(
        provenance='Owned PCM/CABAC B pictures, MP4 and independent scalar reference/weight oracle; no private media, external codec, network or test-time generation.',
        decode_order=[0,2,1],presentation_order=[0,1,2],l0=['past','current','future'],l1=['future','past','current'],weights=WEIGHTS,
        cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
