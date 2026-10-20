"""Owned depth-IV B pictures, reordered references and independent pixel oracle."""
import json,struct
from functools import lru_cache
import generate_hevc_depth_iv as p
from generate_hevc_layered_sps import ue
from generate_hevc_layered_pps import se
from generate_hevc_colour_plane_fixtures import packed,escape
from generate_hevc_layered_reconstruction import parameter_ids
from generate_hevc_layered_slices import header_pps
from generate_hevc_cqt import banks,context,merge_index_bins,terminal_zero,reference
from generate_hevc_wedge_tables import OUT
from hevc_fixture_mp4 import mux
WEIGHTS=[[(1,2),(2,-3),(-1,7)],[(3,-1),(1,4),(0,9)]]
@lru_cache(None)
def parameters(size,enabled,direct,weighted):
    movie=p.movie(size,enabled,False,0,direct)
    pos=movie.index(b'hvcC')+4;config=movie[pos:];at=23;units=[]
    for _ in range(config[22]):
        count=int.from_bytes(config[at+1:at+3],'big');at+=3
        for _ in range(count):
            n=int.from_bytes(config[at:at+2],'big');at+=2
            units.append(config[at:at+n]);at+=n
    if weighted:
        units=[n for n in units if not((int.from_bytes(n[:2],'big')>>9)&63==34 and(int.from_bytes(n[:2],'big')>>3)&63==8)]
        units.append(parameter_ids(header_pps(8,0,True,constrained=direct),0,[3,3]))
    return units

def header(layer,poc,data,stype,negative=(),positive=(),weighted=False,iv=False):
    value='1'+('0'if poc==0 else'')+ue({0:0,3:1,5:2,8:3}[layer])+ue(stype)+'1'
    if layer or poc:value+=f'{poc:08b}'
    if poc:
        value+='0'+ue(len(negative))+ue(len(positive))
        for distances in [negative,positive]:
            previous=0
            for distance in distances:value+=ue(distance-previous-1)+'1';previous=distance
        value+=ue(0)+ue(0)+'0' # no LT entries, no temporal MVP
    if layer in [5,8]:value+='100'if layer==8 and stype==0 else'0'
    value+='00'if layer==0 else'0'
    if stype!=2:
        if layer==8:value+='1'+ue(2)+ue(2) # three active references in both lists
        else:value+='0'
        if len(negative)+len(positive)+(1 if layer==8 else 0)>1:value+='00'
        value+='0' # mvd_l1_zero_flag
        value+='0' # cabac_init_flag
        if weighted:
            value+=ue(1)
            for entries in WEIGHTS:
                value+='1'*len(entries)
                for weight,offset in entries:value+=se(weight-2)+se(offset)
        value+=ue(int(iv)) # five candidates, including Annex I NumExtra
    value+=se(0)+ue(1)+'00000000'+'1';value+='0'*((-len(value))%8)
    return (((19 if poc==0 else 1)<<9)|(layer<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

def source_b(size,mode):
    b,c=banks(2);c['mvd1']=context(b,198);columns=size//32
    for y in range(columns):
        for x in range(columns):
            b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False);b.decision(c['part0'],True)
            b.decision(c['merge'],False);b.decision(c['ip0'],mode=='bi')
            if mode!='bi':b.decision(c['ip4'],mode=='l1')
            for list,mv in [(0,1),(1,-2)]:
                if mode=='l0'and list==1 or mode=='l1'and list==0:continue
                value=mv if x==0 and y==0 else 0
                b.decision(c['mvd'],False);b.decision(c['mvd'],value!=0)
                if value:
                    b.decision(c['mvd1'],abs(value)>1)
                    if abs(value)>1:b.bypass(False);b.bypass(False) # abs-2=0, order 1
                    b.bypass(value<0)
                b.decision(c['mvp'],False)
            b.decision(c['root'],False)
            if y*columns+x+1<columns**2:terminal_zero(b)
    return b.finish()
def target_b(size,index):
    b,c=banks(2);columns=size//32
    for y in range(columns):
        for x in range(columns):
            b.decision(c['split0'],False);b.decision(c[f'skip{int(x>0)+int(y>0)}'],True)
            merge_index_bins(b,c,index,5)
            if y*columns+x+1<columns**2:terminal_zero(b)
    return b.finish()
def movie(size,enabled,direct,weighted,mode,index,intra):
    params=parameters(size,enabled,direct,weighted)
    tex,_=reference(size,5,True,flat=True);d0,_=reference(size,5,False);d2,_=reference(size,5,False,bias=97)
    units=[[header(layer,0,tex if layer==0 else d0,2)for layer in [0,3,5,8]],
        [header(layer,2,tex if layer==0 else d2,2,negative=[2])for layer in [0,3,5,8]],
        [header(0,1,tex,2,negative=[1],positive=[1]),
         header(3,1,d0 if intra else source_b(size,mode),2 if intra else 0,negative=[1],positive=[1]),
         header(5,1,d0,2,negative=[1],positive=[1]),
         header(8,1,target_b(size,index),0,negative=[1],positive=[1],weighted=weighted,iv=enabled)]]
    stream=b''.join(b'\x00\x00\x00\x01'+n for n in params+[n for au in units for n in au])
    return mux(stream,8,size,size,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])

def oracle(size,enabled,mode,index,intra,weighted):
    # Reference labels in the constructed lists, independently of native maps.
    l0=['past','current','future'];l1=['future','past','current']
    iv=enabled and not intra
    if iv and index==0:
        refs=[0,0]if mode=='bi'else([0,1]if mode=='l0'else[2,0])
        motion=[1,-2]if mode=='bi'else([1,1]if mode=='l0'else[-2,-2])
    else:
        slot=index-1 if iv else index
        refs=[slot if slot<3 else 0]*2;motion=[0,0]
    source=[];target=[]
    for y in range(size):
        for x in range(size):
            def picture(label,row):
                row=min(size-1,max(0,row))
                return(x*7+row*11+37+(97 if label=='future'else 0))%256
            def current(row):
                if intra:return picture('past',row)
                a=picture('past',row+1);b=picture('future',row-2)
                return(a+b+1)//2 if mode=='bi'else(a if mode=='l0'else b)
            def sample(label,mv):return current(y+mv)if label=='current'else picture(label,y+mv)
            source.append(current(y))
            a=sample(l0[refs[0]],motion[0]);b=sample(l1[refs[1]],motion[1])
            if weighted:
                w0,o0=WEIGHTS[0][refs[0]];w1,o1=WEIGHTS[1][refs[1]]
                value=(w0*a+w1*b+((o0+o1+1)*2))//4
            else:value=(a+b+1)//2
            target.append(min(255,max(0,value)))
    return refs,motion,source,target

def generate():
    files={};cases=[];pixels=bytearray()
    specifications=[]
    for size,direct in [(32,False),(128,False),(128,True)]:
        for enabled in [False,True]:
            for weighted in [False,True]:
                for mode in ['bi','l0','l1']:
                    for index in ([0,1,2]if size==32 else[0]):specifications.append((size,enabled,direct,weighted,mode,index,False))
    for enabled in [False,True]:
        for weighted in [False,True]:specifications.append((32,enabled,False,weighted,'bi',0,True))
    for size,enabled,direct,weighted,mode,index,intra in specifications:
        name=f'depth-iv-b-{size}-'+('on'if enabled else'off')+f'-{mode}-merge{index}'+('-weighted'if weighted else'')+('-direct'if direct else'')+('-intra'if intra else'')+'.mp4'
        files[name]=movie(size,enabled,direct,weighted,mode,index,intra)
        refs,motion,source,target=oracle(size,enabled,mode,index,intra,weighted)
        offsets=[]
        for values in [source,target]:offsets.append(len(pixels)//2);pixels.extend(struct.pack('<'+'H'*len(values),*values))
        cases.append(dict(file=name,size=size,enabled=enabled,direct=direct,weighted=weighted,mode=mode,index=index,intra=intra,references=refs,motion=motion,offsets=offsets))
    files['depth-iv-b-oracles.y16']=bytes(pixels)
    files['depth-iv-b-manifest.json']=(json.dumps(dict(provenance='Owned PCM/CABAC B-source and target, I0/I2/B1 decode order, independently labeled past/future/current reference and scalar weighted/unweighted pixel oracles. No private source, external codec, network or test-time generation.',weights=WEIGHTS,cases=cases),separators=(',',':'))+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
