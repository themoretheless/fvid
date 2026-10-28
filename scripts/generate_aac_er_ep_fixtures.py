"""Owned ER-LC/LTP/LD EP2 wrappers over saved synthetic sources, offline only."""
import copy,json
from pathlib import Path
from generate_aac_ep_multiclass_fixtures import specs,partition,chain,encode,crc,header,interleave,window,octet_stage
from generate_aac_eld_fixtures import mkv
from generate_he_aac_packet_fixtures import field,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
SOURCE=ROOT/'tests/fixtures/playback-errors'
OUT=SOURCE/'aac-er-ep'
SOURCES=['aac-er-lc','aac-er-ltp','aac-ld-filterbank','aac-er-ltp-resilient','aac-hcr','aac-rvlc','aac-er-sections','aac-ld-stereo']

def prefix(asc):
    bits=''.join(f'{b:08b}'for b in asc)
    aot=int(bits[:5],2);assert aot in [17,19,23]
    assert int(bits[5:9],2)==6 and int(bits[9:13],2)!=0
    at=16+(14 if bits[14]=='1'else 0)
    if bits[15]=='1':at+=4
    assert bits[at:at+2]=='00' and len(bits)-at-2<=7 and set(bits[at+2:])<=set('0'),asc.hex()
    return aot,bits[:at]+'10'

def protect(raw,sets,kind,mode,i,grouped):
    message=''.join(f'{v:08b}'for v in raw);selected=i%3;s=sets[selected];classes=copy.deepcopy(s['classes']);order=s['order']
    parts=partition(message,classes,'rs'if grouped else 'bit-chain'if all(c['fec']==0 for c in classes)else 'mixed')
    bad_target=next((k for k in order if parts[k]and classes[k]['crc']),None)
    payloads=[]
    for k,c in enumerate(classes):
        parity=crc(parts[k],c['crc'])if parts[k]else ''
        if mode=='bad-crc'and i==3 and k==bad_target:
            assert parity;parity=str(1-int(parity[0]))+parity[1:]
        payloads.append(parts[k]+parity)
    indices=[k for k in order if classes[k]['fec']==0 and parts[k]]
    coded=['']*len(classes)
    for k,data in zip(indices,chain([payloads[k]for k in indices],[classes[k]['rate']for k in indices],[classes[k]['terminated']for k in indices])):coded[k]=data
    if grouped:
        members=[]
        for k in order:
            members.append(k)
            if classes[k]['fec']==1:
                joined=''.join(payloads[j]for j in members)
                coded[members[0]]=''.join(f'{v:08b}'for v in encode(packed(joined),classes[k]['rate']))if joined else ''
                members=[]
        assert not members
    else:
        for k,c in enumerate(classes):
            if c['fec']and parts[k]:coded[k]=''.join(f'{v:08b}'for v in encode(packed(payloads[k]),c['rate']))
    if mode=='corrected':
        for k,c in enumerate(classes):
            if coded[k]and(c['fec']or c['rate']==24):coded[k]=str(1-int(coded[k][0]))+coded[k][1:]
    if mode=='truncated'and i==3:
        target=next(k for k in order if len(coded[k])>=8);coded[target]=coded[target][:-8]
    make=lambda stuff:''.join(field(len(parts[k]),classes[k]['length'][1])if classes[k]['length'][0]=='band'and classes[k]['length'][1]else ''for k in order)+field(stuff,3)
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
    assert len(payload)%8==0
    return packed(payload),selected

def generate():
    files={};cases=[];malformed=[];blob=bytearray();plain=bytearray()
    for stem in SOURCES:
        m=json.loads((SOURCE/(stem+'.json')).read_text());source=(SOURCE/(stem+'-packets.bin')).read_bytes();gold=(SOURCE/(stem+'-reference.f32le')).read_bytes()
        for original in m['cases']:
            aot,core=prefix(bytes.fromhex(original['asc']));base=stem+'-'+original['name'];channels=original['channels'];n=original['n']
            golden=base+'.f32le';a=original['reference_offset'];files[golden]=gold[a:a+original['reference_bytes']]
            assert len(files[golden])==len(original['frames'])*n*channels*4
            variants=[('chain',3,'bit-chain',1,0,'clean',False),('mixed',3,'mixed',2,4,'declared',False),('group',7,'rs',2,1,'clean',True),('corrected',7,'rs',0,0,'corrected',True)]
            if len(original['frames'])>3:variants +=[(mode,3,'mixed',2,4,mode,False)for mode in ['bad-crc','truncated']]
            for label,count,family,kind,switch,mode,grouped in variants:
                name=base+'-'+label
                # CRC/truncation rollback controls must have identical ASC.
                config_mode='declared'if label in ['mixed','bad-crc','truncated']else mode
                sets,ep=specs(count,family,kind,switch,config_mode,grouped);asc=packed(core+ep);files[name+'.asc']=asc;frames=[];packets=[]
                for i,row in enumerate(original['frames']):
                    a=row['offset'];raw=source[a:a+row['bytes']];packet,selected=protect(raw,sets,kind,mode,i,grouped)
                    frames.append(dict(offset=len(blob),bytes=len(packet),choice=selected,plain_offset=len(plain),plain_bytes=len(raw)));blob.extend(packet);plain.extend(raw);packets.append(packet)
                case=dict(name=name,n=n,slots=original.get("slots",n//64),bands=original.get("bands",32),channels=channels,rate=24000,aot=aot,asc=asc.hex(),plain_asc=original['asc'],source=stem,source_name=original['name'],frames=frames,container_rate=24000,container_frame_samples=n,samples=n*len(frames),pcm_offset=0,golden=golden,sets=3,configs=sets,interleave=kind,switch=switch)
                filename=name+'.mp4';files[filename]=video_fixture([case],blob,channels,filename=filename,write=False)['data'];case['video']=filename;files[name+'.mka']=mkv(asc,packets,24000,n,channels);case['matroska']=name+'.mka'
                if label in ['bad-crc','truncated']:
                    case['error']='EP class CRC mismatch'if label=='bad-crc'else 'EP RS class/packet length mismatch';case['good']=base+'-mixed';malformed.append(case)
                else:cases.append(case)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own saved ER-LC/LTP/LD spectra, entropy and independent PCM; owned EP2 SRCPC/RS/CRC polynomials and recursive matrices. No private media, external codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
