"""Original NxN mixed ordinary/wedge/contour CRA CU and pixel oracle, offline."""
import struct
from generate_hevc_depth_dc import Encoder
from generate_hevc_layered_sps import sps
from generate_hevc_3d_sps import payload
from generate_hevc_wedge_tables import OUT,generate as wedges
from generate_he_aac_packet_fixtures import DEST
SCHEDULES=['owco','cowo','wocw','wcwc']

def references(image,x,y,n,size):
    width,height=(size,size)if isinstance(size,int)else size
    def at(xx,yy):return image[yy*width+xx] if 0<=xx<width and 0<=yy<height else None
    chain=[at(x-1,y+i)for i in reversed(range(2*n))]+[at(x-1,y-1)]+[at(x+i,y-1)for i in range(2*n)]
    previous=next((v for v in chain if v is not None),128)
    filled=[]
    for v in chain:
        if v is not None:previous=v
        filled.append(previous)
    return filled[2*n+1:],list(reversed(filled[:2*n])),filled[2*n]

def prediction(image,texture,mode,x,y,dlt,n,size,ordinary_mode=1,texture_stride=64):
    top,left,corner=references(image,x,y,n,size)
    if mode=='o':
        if ordinary_mode in (10,26):
            out=[top[xx]if ordinary_mode==26 else left[yy]for yy in range(n)for xx in range(n)]
            for i in range(n):
                value=top[0]+((left[i]-corner)>>1)if ordinary_mode==26 else left[0]+((top[i]-corner)>>1)
                out[i*n if ordinary_mode==26 else i]=max(0,min(255,value))
            return out
        dc=(sum(top[:n])+sum(left[:n])+n)//(2*n)
        out=[dc]*(n*n)
        # Ordinary intra-DC boundary filtering; DIM edges remain unfiltered.
        out[0]=(left[0]+2*dc+top[0]+2)//4
        for i in range(1,n):
            out[i]=(top[i]+3*dc+2)//4
            out[i*n]=(left[i]+3*dc+2)//4
        return out
    if mode=='w':mask=list(wedges()[f'{n}.bin'][:n*n])
    else:
        threshold=sum(texture[(y+dy)*texture_stride+x+dx]for dx,dy in [(0,0),(n-1,0),(0,n-1),(n-1,n-1)])//4
        mask=[texture[(y+dy)*texture_stride+x+dx]>threshold for dy in range(n)for dx in range(n)]
    vertical=mask[0]!=mask[n-1];horizontal=mask[0]!=mask[(n-1)*n]
    if vertical==horizontal:
        lt=(left[0]+top[0])//2
        br=(left[n-1]+top[n-1])//2 if horizontal else top[-1] if abs(top[0]-top[-1])>abs(left[0]-left[-1]) else left[-1]
    else:
        lt=top[(n-1)//2]if horizontal else left[(n-1)//2]
        br=left[n-1]if horizontal else top[n-1]
    out=[]
    for bit in mask:
        pred=lt if bool(bit)==bool(mask[0]) else br
        offset=10 if bit else -10
        if dlt:
            # Nearest populated index, midpoint ties upward; Clip1Y is 8-bit.
            index=max(0,min(255,min(31,(pred+4)//8)+offset))
            pred=index*8 if index<32 else 0
        else:pred+=offset
        out.append(pred)
    return out

def generate(schedule='owco',residual=False,size=32,invalid_split=False,ordinary_mode=1,ordinary_split=False):
    assert schedule in SCHEDULES and size in (8,32)
    n=size//2
    assert ordinary_mode in (1,10,26) and (not ordinary_split or size==32)
    assert not invalid_split or (size==32 and not residual)
    flags=[False]*14;flags[9]=flags[10]=True
    b=Encoder(0,26)
    for value in [184,154,154,184,138,111,110 if n==4 else 125,110 if n==4 else 125,92,138,138,125,125]:
        pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
        b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
    b.decision(3,False) # minimum CU: PART_NxN, no CU split flag
    ordinary_codes=[];published=[]
    for i,mode in enumerate(schedule):
        b.decision(4,mode=='o')
        if mode=='o':
            a=published[i-1]if i%2 else 1
            c=published[i-2]if i>=2 else 1
            if a==c:
                candidates=[0,1,26]if a<2 else[a,2+(a+29)%32,2+(a-1)%32]
            else:candidates=[a,c,0 if a and c else 1 if a!=1 and c!=1 else 26]
            mpm=ordinary_mode in candidates
            value=candidates.index(ordinary_mode)if mpm else ordinary_mode-sum(v<ordinary_mode for v in candidates)
            b.decision(6,mpm);ordinary_codes.append((mpm,value));published.append(ordinary_mode)
        else:b.decision(5,mode=='c');published.append(1)
        if mode=='w':
            for _ in range(7 if n==4 else 9):b.bypass(False)
    for mpm,value in ordinary_codes:
        if mpm:
            b.bypass(value>0)
            if value:b.bypass(value==2)
        else:
            for bit in f'{value:05b}':b.bypass(int(bit))
    for mode in schedule:
        if mode!='o':
            for negative in [True,False]:b.absolute(10);b.bypass(negative)
    levels=[0,64,-64,64]if residual else [0]*4
    def coefficients(level,child=False):
        b.decision(8,level!=0)
        if level:
            b.decision(14 if child else 9,False);b.decision(15 if child else 10,False)
            b.decision(11,True);b.decision(12,True);b.bypass(level<0)
            for bit in '11111111011011':b.bypass(int(bit))
    signalled_invalid=False
    for mode,level in zip(schedule,levels):
        invalid=invalid_split and not signalled_invalid and mode!='o'
        split=invalid or (ordinary_split and mode=='o')
        if n>4:b.decision(7,split)
        if split:
            signalled_invalid|=invalid
            for _ in range(4):
                b.decision(13,False)
                coefficients(0 if invalid else level,True)
        else:coefficients(level)
    texture=(DEST/'hevc-pcm-reference-rext8.yuv').read_bytes()[:4096]
    files={'contour-picture.sps':sps(multilayer=True,three_d=payload(flags,sub=(2,2)if size==32 else(0,0)),coding_blocks=(5,5)if size==32 else(3,4),transform_blocks=(2,5)if size==32 else(2,4)),
        'contour-picture.cabac':b.finish()}
    for dlt in [False,True]:
        image=[None]*(size*size)
        for i,mode in enumerate(schedule):
            x=(i%2)*n;y=(i//2)*n
            blocks=[(x,y,n)]
            if ordinary_split and mode=='o':blocks=[(x+dx*8,y+dy*8,8)for dx,dy in [(0,0),(1,0),(0,1),(1,1)]]
            for bx,by,bn in blocks:
                values=prediction(image,texture,mode,bx,by,dlt,bn,size,ordinary_mode)
                level=levels[i]
                if bn==4:
                    # Independent DST4 DC impulse using normative first basis row.
                    scaled=(level*16*51*16+16)>>5;basis=[29,55,74,84]
                    residual_samples=[((max(-32768,min(32767,(scaled*basis[yy]+64)>>7))*basis[xx]+2048)>>12)for yy in range(4)for xx in range(4)]
                else:
                    # Independent two-pass DCT impulse at QP26, bit depth8.
                    shift=8+(bn.bit_length()-1)+10-15
                    scaled=(level*16*51*16+(1<<(shift-1)))>>shift
                    first=max(-32768,min(32767,(scaled*64+64)>>7))
                    residual_samples=[(first*64+2048)>>12]*(bn*bn)
                for yy in range(bn):
                    for xx in range(bn):image[(by+yy)*size+bx+xx]=max(0,min(255,values[yy*bn+xx]+residual_samples[yy*bn+xx]))
        files['contour-dlt.y16'if dlt else'contour-picture.y16']=struct.pack(f'<{size*size}H',*image)
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for schedule in SCHEDULES:
        for residual,label in [(False,'zero'),(True,'mixed')]:
            for n,b in generate(schedule,residual).items():
                name='nxn-picture.sps'if n.endswith('.sps')else f'nxn-{schedule}-{label}-{n.removeprefix("contour-")}'
                (OUT/name).write_bytes(b)

    for schedule in SCHEDULES:
        for residual,label in [(False,'zero'),(True,'mixed')]:
            for n,b in generate(schedule,residual,8).items():
                name='nxn4-picture.sps'if n.endswith('.sps')else f'nxn4-{schedule}-{label}-{n.removeprefix("contour-")}'
                (OUT/name).write_bytes(b)

    for schedule in ['owco','wcwc']:
        (OUT/f'nxn-{schedule}-split.cabac').write_bytes(generate(schedule,invalid_split=True)['contour-picture.cabac'])
