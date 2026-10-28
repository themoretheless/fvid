"""Own ELD ESC semantic reconstruction and EP3 movies; explicit/offline."""
import json
from pathlib import Path
from generate_aac_eld_fixtures import generate as eld,mkv,specifications
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ep_class_codes_fixtures import chain
from generate_aac_ep_optional_fixtures import header
from generate_aac_ep_playback_fixtures import crc
from generate_aac_sbr_dsp_fixtures import header as sbr_header
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-esc'
def configuration(base,patterns,orders):
    ep=field(len(patterns),8)+'00'+'001'+'001'
    for pattern,order in zip(patterns,orders):
        ep+=field(len(pattern),6)
        for index in range(len(pattern)):
            ep+='100'+'00'+field(index==order[-1],1)+'0'+field(15,4)+field(16,5)+field(8,5)
        ep+='1'+''.join(field(index,6)for index in order)
    ep+='1'+field(24,5)+field(16,5)
    prefix=field(31,5)+field(7,6)+frequency(base['rate'])+field(base['configuration'],4)+field(base['n']==480,1)+field(base['flags'],3)+'0'+'0000'
    return packed(prefix+'11'+ep+'1'),packed(prefix+'10'+ep),packed(prefix+'11'+ep+'0'),ep
def protect(row,patterns,orders,mutation=None):
    selected=patterns.index(tuple(i['category']for i in row['esc']));order=orders[selected]
    messages=[row['esc'][k]['bits']for k in order]
    if mutation=='short-side':
        index=next(i for i,k in enumerate(order)if row['esc'][k]['category']==1)
        messages[index]=messages[index][:7]
    payloads=[bits+crc(bits,8)if bits else ''for bits in messages]
    if mutation=='crc':
        index=next(i for i,bits in enumerate(messages)if bits)
        payloads[index]=payloads[index][:-1]+str(1-int(payloads[index][-1]))
    encoded=chain([payload for payload in payloads if payload],[16 for payload in payloads if payload],[k==order[-1]for k,payload in zip(order,payloads)if payload])
    body=''.join(encoded);selector=header(field(selected,(len(patterns)-1).bit_length())if len(patterns)>1 else '',True)
    attrs=''.join(field(len(bits),15)for bits in messages)
    stuff=(-len(selector)-len(header(attrs+'000',True))-len(body))%8
    wire=selector+header(attrs+field(stuff,3),True)+body+'0'*stuff
    assert len(wire)%8==0
    return packed(wire),selected
def generate():
    specs=specifications()+[(n,24000,2,tool,flags,1,False)for n in [480,512]for tool in ['tns','pns','correlated-pns','intensity']for flags in range(1,8)]
    originals=eld(True,specs,include_category_bits=True)
    m=json.loads(originals['manifest.json']);files={};blob=bytearray();raw=bytearray();protected=bytearray();cases=[];malformed=[]
    for base in m['cases']:
        patterns=list(dict.fromkeys(tuple(i['category']for i in row['esc'])for row in base['frames']))
        orders=[list(range(len(pattern)))[::-1]if selected%2 else list(range(len(pattern)))for selected,pattern in enumerate(patterns)]
        asc,ep2,reserved,ep=configuration(base,patterns,orders)
        variants=['clean']+(['short-side','crc']if base['rate']==24000 and base['configuration']in[1,2]and(base['tool']=='base'or base['flags']==7)else [])
        for variant in variants:
            rows=[];packets=[]
            for index,row in enumerate(base['frames']):
                instances=[]
                for instance in row['esc']:
                    data=packed(instance['bits'])
                    instances.append(dict(category=instance['category'],offset=len(blob),bytes=len(data),bits=len(instance['bits'])))
                    blob.extend(data)
                original=originals['packets.bin'][row['offset']:row['offset']+row['bytes']]
                packet,selected=protect(row,patterns,orders,variant if variant!='clean'and index==3 else None)
                packets.append(packet)
                rows.append(dict(instances=instances,plain_offset=len(raw),plain_bytes=len(original),offset=len(protected),bytes=len(packet),choice=selected))
                raw.extend(original);protected.extend(packet)
            name=base['name']+(''if variant=='clean'else '-'+variant)
            movie_case=dict(name=name,asc=asc.hex(),frames=[dict(offset=r['offset'],bytes=r['bytes'])for r in rows],container_rate=base['rate'],container_frame_samples=base['n'],samples=base['n']*len(rows),pcm_offset=0,slots=base['n']//32,bands=32)
            files[name+'.mp4']=video_fixture([movie_case],protected,base['channels'],filename=name+'.mp4',write=False)['data']
            files[name+'.mka']=mkv(asc,packets,base['rate'],base['n'],base['channels'])
            case=dict(name=name,plain_asc=base['asc'],asc=asc.hex(),ep2_asc=ep2.hex(),reserved_asc=reserved.hex(),channels=base['channels'],n=base['n'],rate=base['rate'],configuration=base['configuration'],flags=base['flags'],frames=rows,video=name+'.mp4',matroska=name+'.mka')
            if base['name']=='n512-c1':
                prefix=field(31,5)+field(7,6)+frequency(24000)+field(1,4)+'0'+'000'+'111'+sbr_header(0,True)+'0000'
                case['ld_sbr_asc']=packed(prefix+'11'+ep+'1').hex()
            if variant=='clean':
                files[name+'.f32le']=originals[name+'.f32le'];files[name+'.asc']=asc;cases.append(case)
            else:
                case.update(good=base['name'],bad_index=3,error='truncated or oversized bit field'if variant=='short-side'else 'EP class CRC mismatch')
                malformed.append(case)
    files['packets.bin']=bytes(protected);files['instances.bin']=bytes(blob);files['plain-packets.bin']=bytes(raw)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own ELD encoder assigns fields to ESC categories 0..4 (ISO table 4.148); independent saved PCM uses direct cosine/window arithmetic. Independent CRC/SRCPC/headers protect each class; short-side negatives retain valid FEC/CRC. No private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
