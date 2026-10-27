"""Owned EP2 bit-class assembly, continuous SRCPC, RS and recursive playback."""
import json
from pathlib import Path
from generate_aac_ep_sets_fixtures import header
from generate_aac_ep_playback_fixtures import crc
from generate_aac_ep_class_codes_fixtures import chain,octet_stage
from generate_aac_ep_interleave_fixtures import stage
from generate_aac_ep_rs_fixtures import encode
from generate_aac_eld_fixtures import generate as eld,mkv
from generate_aac_sbr_dsp_fixtures import header as sbr_header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-multiclass'
PROFILES=[(0,0),(1,0)]+[(2,s)for s in range(4)]
def window(n,h):return 3 if n<=2 else len(h)if n<=16 else 28
def interleave(x,y,w):return ''.join(stage(list(x),list(y),w))
def specs(number,family,kind,switch,mode):
    sets=[]
    for selected,count in enumerate([number,max(1,number-1),number]):
        order=list(range(count))
        if (selected+switch+kind)%3==1:order=order[1:]+order[:1]
        if (selected+switch+kind)%3==2:order=order[::-1]
        if mode=='chain-corrected':order=list(range(count))
        classes=[]
        for i in range(count):
            fec=1 if family=='rs'or family=='mixed'and i%2==0 else 0
            requested=(i+selected)%4 if switch==4 else switch
            width=([8,16,24,32][i%4]if fec else [1,4,8,16][i%4])if family=='mixed'else [0,8,16,24,32][i%5]if family in ['rs','chain']else i%17
            classes.append(dict(fec=fec,rate=(i%4+1)if fec else ((1 if mode=='chain-corrected'else 0)if i==0 else 24)if family=='chain'else (i*7+8+selected)%25,crc=width,terminated=family not in ['chain','bit-chain']or i==count-1,interleave=1 if fec and requested==2 else requested,length=('fixed',8)if count>1 and i==0 else ('fixed',0)if count>=4 and i==1 else ('band',15)))
        for c in classes:
            c['rate_escape']=mode=='dynamic'and c['fec']==0
            c['crc_escape']=mode=='dynamic'
        last=order[-1]
        if classes[last]['length'][0]!='fixed'and mode not in ['truncated','declared']:classes[last]['length']=('band',0)
        sets.append(dict(classes=classes,order=order))
    result=field(3,8)+field(kind,2)+'001'+'001'
    for s in sets:
        result+=field(len(s['classes']),6)
        for c in s['classes']:
            escaped=c['length'][0]=='band';result+=field(escaped,1)+field(c['rate_escape'],1)+field(c['crc_escape'],1)+field(c['fec'],2)
            if c['fec']==0:result+=field(c['terminated'],1)
            if kind==2:result+=field(c['interleave'],2)
            result+='0'+field(c['length'][1],4 if escaped else 16)
            if not c['rate_escape']:result+=field(c['rate'],7 if c['fec']else 5)
            if not c['crc_escape']:result+=field(17 if c['crc']==24 else 18 if c['crc']==32 else c['crc'],5)
        result+='1'+''.join(field(i,6)for i in s['order'])
    return sets,result+'1'+field(24,5)+field(16,5)
def partition(message,classes,family):
    result=[];at=0
    for i,c in enumerate(classes):
        if c['length'][0]=='fixed':n=c['length'][1]
        else:
            remaining=sum(d['length']!=('fixed',0)for d in classes[i:]);unit=1 if family in ['srcpc','bit-chain']else 8
            n=((len(message)-at)//unit//remaining)*unit
        assert n>=0 and at+n<=len(message)
        result.append(message[at:at+n]);at+=n
    assert at==len(message),(len(message),at,classes)
    return result

def generate():
    files={};cases=[];malformed=[];blob=bytearray();plain_blob=bytearray()
    core=eld(True,[(n,24000,c,'base',0,0,False)for n in [480,512]for c in [1,2]])
    ldroot=ROOT/'tests/fixtures/playback-errors/aac-eld-sbr';lm=json.loads((ldroot/'manifest.json').read_text());lp=(ldroot/'packets.bin').read_bytes()
    for ld in [False,True]:
        for base in json.loads(core['manifest.json'])['cases']:
            n=base['n'];channels=base['channels'];original=next(c for c in lm['cases']if c['name']==f'n{n}-c{channels}-x2-crc1')if ld else base;source=lp if ld else core['packets.bin'];rate=48000 if ld else 24000;frame_samples=n*(2 if ld else 1)
            golden=f'n{n}-c{channels}-ld{int(ld)}.f32le';files[golden]=(ldroot/(original['name']+'.f32le')).read_bytes()if ld else core[golden.replace(f'-ld{int(ld)}','')]
            prefix=field(31,5)+field(7,6)+frequency(24000)+field(channels,4)+field(n==480,1)+'000'+field(ld,1)+('11'+sbr_header(0,True)if ld else '')+'0000'+'10'
            modes=[(count,family,t,s,'clean')for count in [2,3,4,7]for family in ['srcpc','chain','rs','mixed']for t,s in PROFILES]
            if n==512 and channels==1:modes +=[(count,family,t,s,'clean')for count in [16,31,63]for family in ['srcpc','mixed']for t,s in PROFILES]
            modes +=[(count,'bit-chain',t,s,'clean')for count in [3,7]for t,s in PROFILES]
            modes +=[(count,family,t,s,'dynamic')for count,family in [(3,'bit-chain'),(7,'mixed')]for t,s in PROFILES]
            modes +=[(7,'mixed',2,4,v)for v in ['clean','corrected','declared','bad-crc','truncated']]
            modes +=[(3,'chain',t,s,'chain-corrected')for t,s in PROFILES]
            modes +=[(3,'mixed',t,s,v)for t,s in PROFILES for v in ['corrected','declared','bad-crc','truncated']]
            for count,family,kind,switch,mode in modes:
                name=f'n{n}-c{channels}-ld{int(ld)}-classes{count}-{family}-i{kind}-s{switch}-{mode}';sets,ep=specs(count,family,kind,switch,mode);asc=packed(prefix+ep);files[name+'.asc']=asc;packets=[];frames=[]
                for i,row in enumerate(original['frames']):
                    a=row['offset'];raw=source[a:a+row['bytes']];message=''.join(f'{v:08b}'for v in raw);selected=i%3;s=sets[selected];classes=[dict(c)for c in s['classes']];order=s['order']
                    for k,c in enumerate(classes):
                        c['rate_code']=(i+k)%8;c['crc_code']=[0,2,6,7][(i+k)%4]if c['fec']else (i+k)%8
                        if c['rate_escape']:c['rate']=[0,3,4,6,8,12,16,24][c['rate_code']]
                        if c['crc_escape']:c['crc']=[0,6,8,10,12,14,16,32][c['crc_code']]
                    parts=partition(message,classes,family);payloads=[]
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
                    if mode=='chain-corrected':
                        assert len(parts[0])==8 and classes[0]['rate']==1 and classes[0]['crc']==0
                        coded[0]=coded[0][:-1]+str(1-int(coded[0][-1]))
                    if mode=='truncated'and i==3:
                        assert len(coded[order[-1]])>=8;coded[order[-1]]=coded[order[-1]][:-8]
                    make=lambda stuff:''.join((field(len(parts[k]),classes[k]['length'][1])if classes[k]['length'][0]=='band'and classes[k]['length'][1]else '')+(field(classes[k]['rate_code'],3)if classes[k]['rate_escape']else '')+(field(classes[k]['crc_code'],3)if classes[k]['crc_escape']else '')for k in order)+field(stuff,3)
                    selector=header(field(selected,2));h=header(make(0));stuff=(-len(selector)-len(h)-sum(map(len,coded)))%8;h=header(make(stuff))
                    if mode=='corrected':selector=str(1-int(selector[0]))+selector[1:];h=h[:-1]+str(1-int(h[-1]))
                    if kind==0:body=''.join(coded[k]for k in order)
                    else:
                        body=''.join(coded[k]for k in order if kind==2 and classes[k]['interleave']==3)
                        for k in reversed(order):
                            c=classes[k];value=c['interleave']
                            if coded[k]and(kind==1 or value in [1,2]):
                                body=''.join(octet_stage(list(coded[k]),list(body)))if c['fec']else interleave(coded[k],body,28 if kind==1 or value==2 else len(coded[k]))
                        if kind==2:body+=''.join(coded[k]for k in order if classes[k]['interleave']==0)
                    payload=selector+h+body+'0'*stuff if kind==0 else interleave(selector,interleave(h,body+'0'*stuff,window(len(make(stuff)),h)),3)
                    assert len(payload)%8==0;packet=packed(payload);frames.append(dict(offset=len(blob),bytes=len(packet),choice=selected,plain_offset=len(plain_blob),plain_bytes=len(raw)));blob.extend(packet);plain_blob.extend(raw);packets.append(packet)
                case=dict(name=name,n=n,slots=n//32,bands=64 if ld else 32,channels=channels,rate=rate,asc=asc.hex(),frames=frames,container_rate=rate,container_frame_samples=frame_samples,samples=frame_samples*12,pcm_offset=0,golden=golden,sets=3,configs=sets,interleave=kind,switch=switch,mask=4 if channels==1 else 3)
                filename=name+'.mp4';files[filename]=video_fixture([case],blob,channels,filename=filename,write=False)['data'];case['video']=filename;files[name+'.mka']=mkv(asc,packets,rate,frame_samples,channels);case['matroska']=name+'.mka'
                if mode in ['bad-crc','truncated']:
                    case['error']='EP class CRC mismatch'if mode=='bad-crc'else 'EP RS class/packet length mismatch'if sets[0]['classes'][sets[0]['order'][-1]]['fec']else 'EP class/packet length mismatch';case['good']=name.replace(mode,'declared'if mode=='truncated'else 'clean');malformed.append(case)
                else:cases.append(case)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain_blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own nonzero ELD/LD-SBR PCM and bit partitions, polynomial continuous SRCPC, RS long division and occupied-cell bit/octet matrices. No private media, external codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
