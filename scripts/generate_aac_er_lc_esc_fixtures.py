"""Own ER-LC EP3 ESC semantic mapping, offline synthetic source tracing."""
import copy,json
from pathlib import Path
from generate_aac_er_lc_fixtures import er_channel
from generate_aac_main_tools_fixtures import ics
from generate_aac_er_ep_fixtures import prefix
from generate_aac_eld_esc_fixtures import configuration,protect
from generate_aac_eld_fixtures import mkv
from generate_he_aac_packet_fixtures import field,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-er-lc-esc'
LAYOUTS={1:[0],2:[1],3:[0,1],6:[0,1,1,3]}

def trace(base,index,independent):
    seq=base['frames'][index]['sequence']if 'sequence'in base['frames'][index]else [0,0,0,0,1,2,2,3,0,0,0,0][index]
    asc=''.join(f'{v:08b}'for v in bytes.fromhex(base['asc']));layout=int(asc[9:13],2);ms=int(base['name'].split('-')[2])
    header=ics(seq,2,False,shape=index%2);wire='';parts=[];ch=0
    for kind in LAYOUTS[layout]:
        width=2 if kind==1 else 1;tag='0000';common=width==2 and not independent
        if width==2:
            shared=tag+field(common,1)+(header+field(ms,2)+('10'if ms==1 else '')if common else '')
            parts.append(dict(category=0,bits=shared));wire+=shared
        for c in range(width):
            values=[[(-1 if (index+w+k+ch+c)%3==0 else 1 if (index+w+k+ch+c)%3==1 else 0)for k in range(8)]for w in range(8 if seq==2 else 1)]
            info=''if common else header;body=er_channel(seq,values,info,bool((index+ch+c)%2));side=8+len(info)+(14 if seq==2 else 18)+2+3
            if width==1:wire+=tag
            wire+=body
            parts.append(dict(category=1,bits=(tag if width==1 else '')+body[:side]))
            if seq!=2:
                parts.append(dict(category=3,bits=body[side:side+19]));side+=19
            parts.append(dict(category=4,bits=body[side:]))
        ch+=width
    parts[-1]['bits']+='0'*(-len(wire)%8)
    return wire,parts

def generate():
    m=json.loads((SOURCE/'aac-er-lc.json').read_text());raw=(SOURCE/'aac-er-lc-packets.bin').read_bytes();gold=(SOURCE/'aac-er-lc-reference.f32le').read_bytes()
    files={};cases=[];bad=[];blob=bytearray();plain=bytearray();instances=bytearray()
    for base in m['cases']:
        for independent in [False]+([True]if base['name'].endswith('-2-0-0')else []):
            stem=base['name']+('-independent'if independent else '')
            frames=[]
            for index,row in enumerate(base['frames']):
                wire,parts=trace(base,index,independent)
                if not independent:assert packed(wire)==raw[row['offset']:row['offset']+row['bytes']],(stem,index)
                frames.append(dict(wire=packed(wire),esc=parts))
            patterns=list(dict.fromkeys(tuple(p['category']for p in row['esc'])for row in frames));orders=[list(range(len(p)))[::-1]if i%2 else list(range(len(p)))for i,p in enumerate(patterns)]
            _,_,_,ep=configuration(dict(rate=24000,configuration=1,n=512,flags=0),patterns,orders)
            _,setup=prefix(bytes.fromhex(base['asc']));setup=setup[:-2]
            asc=packed(setup+'11'+ep+'1');ep2=packed(setup+'10'+ep);reserved=packed(setup+'11'+ep+'0')
            files[stem+'.asc']=asc;start=base['reference_offset'];files[stem+'.f32le']=gold[start:start+base['reference_bytes']]
            selected=independent or base['name'].endswith('-1-0-0')or base['name'].endswith('-6-2-1')
            for variant in ['clean']+(['short-side','short-spectrum','crc','alignment']if selected else []):
                label=stem if variant=='clean'else stem+'-'+variant;rows=[];packets=[]
                for index,source in enumerate(frames):
                    row=copy.deepcopy(source)
                    if variant!='clean'and index==3:
                        if variant=='short-side':next(p for p in reversed(row['esc'])if p['category']==1)['bits']=''
                        if variant=='short-spectrum':next(p for p in reversed(row['esc'])if p['category']==4)['bits']=''
                        if variant=='alignment':
                            # This source's logical syntax is not byte aligned.
                            wire,_=trace(base,index,independent);assert len(wire)%8
                            p=row['esc'][-1];p['bits']=p['bits'][:-1]+'1'
                    encoded,choice=protect(row,patterns,orders,'crc'if variant=='crc'and index==3 else None)
                    parts=[]
                    for item in row['esc']:
                        packed_bits=packed(item['bits']);parts.append(dict(category=item['category'],offset=len(instances),bytes=len(packed_bits),bits=len(item['bits'])));instances.extend(packed_bits)
                    rows.append(dict(offset=len(blob),bytes=len(encoded),choice=choice,plain_offset=len(plain),plain_bytes=len(source['wire']),instances=parts));blob.extend(encoded);plain.extend(source['wire']);packets.append(encoded)
                c=dict(name=label,n=base['n'],channels=base['channels'],rate=24000,configuration=int(''.join(f'{v:08b}'for v in bytes.fromhex(base['asc']))[9:13],2),flags=0,asc=asc.hex(),plain_asc=base['asc'],ep2_asc=ep2.hex(),reserved_asc=reserved.hex(),frames=rows,golden=stem+'.f32le',container_rate=24000,container_frame_samples=base['n'],samples=base['n']*12,pcm_offset=0,slots=base['n']//64,bands=32,video=label+'.mp4',matroska=label+'.mka')
                files[c['video']]=video_fixture([c],blob,base['channels'],filename=c['video'],write=False)['data'];files[c['matroska']]=mkv(asc,packets,24000,base['n'],base['channels'])
                if variant=='clean':cases.append(c)
                else:c.update(good=stem,bad_index=3,error='EP class CRC mismatch'if variant=='crc'else'nonzero AAC ESC alignment bits'if variant=='alignment'else'truncated or oversized bit field');bad.append(c)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain);files['instances.bin']=bytes(instances)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own saved tagged ER-LC sources, independently traced table 4.148 fields and polynomial EP protection. Sparse direct IMDCT/window/TNS numerical references, independent-window stereo shares the matching MS=0 reference. No private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
