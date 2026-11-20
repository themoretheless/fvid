"""Owned EP2 bit-class assembly, continuous SRCPC, RS and recursive playback."""
import json,copy
from pathlib import Path
from generate_aac_ep_multiclass_fixtures import specs as base_specs,partition
from generate_aac_ep_playback_fixtures import crc,protected_header
from generate_aac_ep_class_codes_fixtures import chain,octet_stage
from generate_aac_ep_interleave_fixtures import stage
from generate_aac_ep_rs_fixtures import encode
from generate_aac_eld_fixtures import generate as eld,mkv
from generate_aac_sbr_dsp_fixtures import header as sbr_header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-optional'
PROFILES=[(0,0),(1,0)]+[(2,s)for s in range(4)]
def header(message,extended):return message*3 if len(message)<3 else protected_header(message,extended)
def window(n,h):return 3 if n<=2 else len(h)if n<=16 else 28
def interleave(x,y,w):return ''.join(stage(list(x),list(y),w))
def optional_specs(number,family,kind,switch,mode,variant,original_count,extended):
    source,_=base_specs(number,family,kind,switch,'declared'if mode=='truncated'else mode)
    sets=[]
    for i in range(original_count):
        item=copy.deepcopy(source[0 if variant=='all'else i%3]);classes=item['classes']
        for k,c in enumerate(classes):
            c['optional']=(True if variant=='all'else k<len(classes)-1 and(variant!='scattered'or k%2==0))
            if variant=='all':c['length']=('band',15)
        if variant=='all'and original_count>1:
            shift=i%len(classes);item['order']=item['order'][shift:]+item['order'][:shift]
            if i%2:item['order'].reverse()
            for k,c in enumerate(classes):
                c['rate']=1+(c['rate']-1+i)%8 if c['fec']else (c['rate']+i)%25
                c['crc']=[0,8,16,24,32][(i+k)%5]if c['fec']else (i+k)%17
        last=item['order'][-1]
        if classes[last]['length'][0]=='band'and mode not in ['declared','truncated']:classes[last]['length']=('band',0)
        sets.append(item)
    counts=[sum(c['optional']for c in s['classes'])for s in sets]
    result=field(len(sets),8)+field(kind,2)+'001'+'001'
    for item in sets:
        result+=field(len(item['classes']),6)
        for c in item['classes']:
            escaped=c['length'][0]=='band';result+=field(escaped,1)+field(c['rate_escape'],1)+field(c['crc_escape'],1)+field(c['fec'],2)
            if c['fec']==0:result+=field(c['terminated'],1)
            if kind==2:result+=field(c['interleave'],2)
            result+=field(c['optional'],1)+field(c['length'][1],4 if escaped else 16)
            if not c['rate_escape']:result+=field(c['rate'],7 if c['fec']else 5)
            if not c['crc_escape']:result+=field(17 if c['crc']==24 else 18 if c['crc']==32 else c['crc'],5)
        result+='1'+''.join(field(k,6)for k in item['order'])
    result+=field(extended,1)+(field(24,5)+field(16,5)if extended else '')
    return sets,counts,result

def selection(sets,counts,i,variant,mode):
    original=[0,len(sets)-1,len(sets)//2][i%3];limit=(1<<counts[original])-1
    patterns=[0,limit,1,(1<<(counts[original]//2)),sum(1<<k for k in range(counts[original])if k%2),limit^(1<<(counts[original]-1))if counts[original]else 0]
    mask=patterns[i%len(patterns)]&limit
    if variant=='all'and mask==limit:mask=limit^1
    if i==3 and mode in ['bad-crc','truncated']:original=0;mask=(1<<counts[0])-1
    if i==3 and mode=='empty':original=0;mask=(1<<counts[0])-1
    value=sum(1<<n for n in counts[:original])+mask
    if i==3 and mode=='bad-selector':value=sum(1<<n for n in counts)
    rank=0;present=[]
    for k,c in enumerate(sets[original]['classes']):
        keep=not c['optional']or not(mask>>rank&1)
        if c['optional']:rank+=1
        if keep:present.append(k)
    return value,original,present

def generate():
    files={};cases=[];malformed=[];blob=bytearray();plain_blob=bytearray()
    core=eld(True,[(n,24000,c,'base',0,0,False)for n in [480,512]for c in [1,2]])
    ldroot=ROOT/'tests/fixtures/playback-errors/aac-eld-sbr';lm=json.loads((ldroot/'manifest.json').read_text());lp=(ldroot/'packets.bin').read_bytes()
    for ld in [False,True]:
        for base in json.loads(core['manifest.json'])['cases']:
            n=base['n'];channels=base['channels'];original=next(c for c in lm['cases']if c['name']==f'n{n}-c{channels}-x2-crc1')if ld else base;source=lp if ld else core['packets.bin'];rate=48000 if ld else 24000;frame_samples=n*(2 if ld else 1)
            golden=f'n{n}-c{channels}-ld{int(ld)}.f32le';files[golden]=(ldroot/(original['name']+'.f32le')).read_bytes()if ld else core[golden.replace(f'-ld{int(ld)}','')]
            prefix=field(31,5)+field(7,6)+frequency(24000)+field(channels,4)+field(n==480,1)+'000'+field(ld,1)+('11'+sbr_header(0,True)if ld else '')+'0000'+'10'
            modes=[(count,'mixed',t,s,'clean','tail',3,True)for count in range(2,10)for t,s in PROFILES]
            modes +=[(5,'bit-chain',t,s,'clean','tail',3,True)for t,s in PROFILES]
            modes +=[(7,'mixed',2,4,'clean','scattered',3,True)]
            modes +=[(5,'mixed',t,s,v,'tail',3,True)for t,s in PROFILES for v in ['dynamic','corrected','declared','bad-selector','bad-crc','truncated']]
            modes +=[(count,'mixed',t,s,'clean','tail',1,True)for count in [2,3]for t,s in [(0,0),(1,0),(2,4)]]
            modes +=[(3,'mixed',t,s,v,'all',1,True)for t,s in [(0,0),(1,0),(2,4)]for v in ['clean','empty']]
            if n==512 and channels==1:
                modes +=[(count,'mixed',t,s,'clean','tail',3,extended)for count in [10,11,12,13,14,15,16,17,31,63]for t,s in [(0,0),(1,0),(2,4)]for extended in [False,True]]
                modes +=[(63,'mixed',0,0,'clean','all',255,extended)for extended in [False,True]]
            for count,family,kind,switch,mode,variant,original_count,extended in modes:
                name=f'n{n}-c{channels}-ld{int(ld)}-classes{count}-{family}-i{kind}-s{switch}-{variant}-p{original_count}-h{int(extended)}-{mode}'
                sets,counts,ep=optional_specs(count,family,kind,switch,mode,variant,original_count,extended)
                total=sum(1<<n for n in counts);selector_width=(total-1).bit_length();asc=packed(prefix+ep);files[name+'.asc']=asc;packets=[];frames=[]
                for i,row in enumerate(original['frames']):
                    a=row['offset'];raw=source[a:a+row['bytes']];message=''.join(f'{v:08b}'for v in raw);value,selected,present=selection(sets,counts,i,variant,mode);s=sets[selected];classes=[dict(c)for c in s['classes']];order=[k for k in s['order']if k in present]
                    for k,c in enumerate(classes):
                        c['rate_code']=(i+k)%8;c['crc_code']=[0,2,6,7][(i+k)%4]if c['fec']else (i+k)%8
                        if c['rate_escape']:c['rate']=[0,3,4,6,8,12,16,24][c['rate_code']]
                        if c['crc_escape']:c['crc']=[0,6,8,10,12,14,16,32][c['crc_code']]
                    parts=['']*len(classes)
                    if present:
                        allocated=partition(message,[classes[k]for k in present],family)
                        for k,part in zip(present,allocated):parts[k]=part
                    payloads=[]
                    for k,c in enumerate(classes):
                        parity=crc(parts[k],c['crc'])if parts[k]else ''
                        if mode=='bad-crc'and i==3 and k==order[-1]:
                            assert parity and parts[k];parity=str(1-int(parity[0]))+parity[1:]
                        payloads.append(parts[k]+parity)
                    indices=[k for k in order if classes[k]['fec']==0 and parts[k]]
                    srcpc=chain([payloads[k]for k in indices],[classes[k]['rate']for k in indices],[classes[k]['terminated']for k in indices]);coded=['']*len(classes)
                    for k,data in zip(indices,srcpc):coded[k]=data
                    for k,c in enumerate(classes):
                        if c['fec']and parts[k]:coded[k]=''.join(f'{v:08b}'for v in encode(packed(payloads[k]),c['rate']))
                    if mode=='corrected':
                        for k,c in enumerate(classes):
                            if coded[k]and(c['fec']or c['rate']==24):coded[k]=str(1-int(coded[k][0]))+coded[k][1:]
                    if mode=='truncated'and i==3:
                        assert len(coded[order[-1]])>=8;coded[order[-1]]=coded[order[-1]][:-8]
                    make=lambda stuff:''.join((field(len(parts[k]),classes[k]['length'][1])if classes[k]['length'][0]=='band'and classes[k]['length'][1]else '')+(field(classes[k]['rate_code'],3)if classes[k]['rate_escape']else '')+(field(classes[k]['crc_code'],3)if classes[k]['crc_escape']else '')for k in order)+field(stuff,3)
                    selector=header(field(value,selector_width),extended);h=header(make(0),extended);stuff=(-len(selector)-len(h)-sum(map(len,coded)))%8;h=header(make(stuff),extended)
                    if mode=='corrected':selector=str(1-int(selector[0]))+selector[1:];h=h[:-1]+str(1-int(h[-1]))
                    if kind==0:body=''.join(coded[k]for k in order)
                    else:
                        body=''.join(coded[k]for k in order if kind==2 and classes[k]['interleave']==3)
                        for k in reversed(order):
                            c=classes[k];value=c['interleave']
                            if coded[k]and(kind==1 or value in [1,2]):
                                body=''.join(octet_stage(list(coded[k]),list(body)))if c['fec']else interleave(coded[k],body,28 if kind==1 or value==2 else len(coded[k]))
                        if kind==2:body+=''.join(coded[k]for k in order if classes[k]['interleave']==0)
                    payload=selector+h+body+'0'*stuff if kind==0 else interleave(selector,interleave(h,body+'0'*stuff,window(len(make(stuff)),h)),window(selector_width,selector))
                    assert len(payload)%8==0;packet=packed(payload);frames.append(dict(offset=len(blob),bytes=len(packet),choice=selected,selector=str(value),present=present,plain_offset=len(plain_blob),plain_bytes=len(raw)));blob.extend(packet);plain_blob.extend(raw);packets.append(packet)
                case=dict(name=name,n=n,slots=n//32,bands=64 if ld else 32,channels=channels,rate=rate,asc=asc.hex(),frames=frames,container_rate=rate,container_frame_samples=frame_samples,samples=frame_samples*12,pcm_offset=0,golden=golden,sets=original_count,class_counts=[len(s["classes"])for s in sets],optional_counts=counts,predefined_sets=str(total),variant=variant,header_extended=extended,interleave=kind,switch=switch,mask=4 if channels==1 else 3)
                filename=name+'.mp4';files[filename]=video_fixture([case],blob,channels,filename=filename,write=False)['data'];case['video']=filename;files[name+'.mka']=mkv(asc,packets,rate,frame_samples,channels);case['matroska']=name+'.mka'
                if mode in ['bad-selector','bad-crc','truncated','empty']:
                    case['error']='EP predefined set index out of range'if mode=='bad-selector'else 'EP class CRC mismatch'if mode=='bad-crc'else 'truncated'if mode=='empty'else 'EP RS class/packet length mismatch'
                    case['raw_empty']=mode=='empty';case['good']=name.replace(mode,'declared'if mode=='truncated'else 'clean');malformed.append(case)
                else:cases.append(case)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain_blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own ELD/LD-SBR PCM, explicitly selected optional subsets, polynomial channel codes and occupied-cell matrices. Large sets use sparse selectors, never expanded combinations. No private media, external codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
