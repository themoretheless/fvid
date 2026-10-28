"""Own EP3 LD-SBR ESC9/10 movies, PCM and header-history controls; offline."""
import copy,json
from pathlib import Path
from generate_aac_eld_sbr_fixtures import generate as ld
from generate_aac_eld_esc_fixtures import configuration,protect
from generate_aac_ep_concatenated_fixtures import config as concatenate,protect as protect_multiple
from generate_aac_eld_fixtures import mkv
from generate_he_aac_packet_fixtures import packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-esc-sbr'
def generate():
    source=ld(True,True);m=json.loads(source['manifest.json']);files={};blob=bytearray();plain=bytearray();information=bytearray();cases=[];bad=[];multiple=[]
    def movie(case,packets):
        files[case['video']]=video_fixture([case],blob,case['channels'],filename=case['video'],write=False)['data']
        files[case['matroska']]=mkv(bytes.fromhex(case['asc']),packets,case['rate'],case['container_frame_samples'],case['channels'])
    for base in m['cases']:
        patterns=list(dict.fromkeys(tuple(p['category']for p in row['esc'])for row in base['frames']))
        orders=[list(range(len(p)))[::-1]if i%2 else list(range(len(p)))for i,p in enumerate(patterns)]
        _,_,_,ep=configuration(dict(base,rate=24000,flags=base.get('resilience',0)),patterns,orders)
        asc=packed(base['asc_bits'][:-2]+'11'+ep+'1')
        selected=base['name']in[f'n{n}-c{c}-x2-crc1'for n in[480,512]for c in[1,2]]or base['name'].endswith('header-switch')
        variants=['clean']+(['short-control','short-values','sbr-crc','ep-crc']if selected else [])
        for variant in variants:
            name=base['name']+(''if variant=='clean'else '-'+variant);rows=[];packets=[]
            bad_index=5 if base['name'].endswith('header-switch')else 3
            for index,original in enumerate(base['frames']):
                row=copy.deepcopy(original)
                if variant!='clean'and index==bad_index:
                    if variant=='short-control':next(p for p in row['esc']if p['category']==9)['bits']=''
                    if variant=='short-values':
                        p=next(p for p in row['esc']if p['category']==10);p['bits']=p['bits'][:1]
                    if variant=='sbr-crc':
                        p=next(p for p in row['esc']if p['category']==9);p['bits']=str(1-int(p['bits'][0]))+p['bits'][1:]
                packet,choice=protect(row,patterns,orders,'crc'if variant=='ep-crc'and index==bad_index else None)
                packets.append(packet);instances=[]
                for item in row['esc']:
                    raw=packed(item['bits']);instances.append(dict(category=item['category'],offset=len(information),bytes=len(raw),bits=len(item['bits'])));information.extend(raw)
                raw=source['packets.bin'][original['offset']:original['offset']+original['bytes']]
                rows.append(dict(offset=len(blob),bytes=len(packet),plain_offset=len(plain),plain_bytes=len(raw),choice=choice,instances=instances))
                blob.extend(packet);plain.extend(raw)
            case=dict(name=name,n=base['n'],rate=base['rate'],channels=base['channels'],configuration=base['configuration'],flags=base.get('resilience',0),asc=asc.hex(),plain_asc=base['asc'],frames=rows,source_frames=1,container_rate=base['rate'],container_frame_samples=base['container_frame_samples'],samples=base['samples'],pcm_offset=0,slots=base['slots'],bands=base['bands'],video=name+'.mp4',matroska=name+'.mka')
            movie(case,packets)
            if variant=='clean':
                files[name+'.asc']=asc;files[name+'.f32le']=source[base['name']+'.f32le'];cases.append(case)
            else:
                error={'short-control':'truncated or oversized bit field','short-values':'truncated or invalid SBR control payload','sbr-crc':'low-delay SBR CRC mismatch','ep-crc':'EP class CRC mismatch'}[variant]
                case.update(good=base['name'],error=error,bad_index=bad_index);bad.append(case)
        # Fixed EP parameters use repeated, independently valid source AUs.
        # These composition controls isolate packet clocks and atomic rollback;
        # the 258 ordinary sources above retain independent numerical PCM.
        if base['name']not in[f'n{n}-c{c}-x{x}-crc1'for n in[480,512]for c in[1,2]for x in[1,2]]:continue
        parts=[p['bits']for p in base['frames'][0]['esc']]
        parts[-1]+='0'*(-sum(map(len,parts))%8)
        unit=packed(''.join(parts));unit_at=len(plain);plain.extend(unit)
        for count in range(2,8):
            sets,ep=concatenate(list(map(len,parts)),count,'srcpc',0,'mixed')
            asc=packed(base['asc_bits'][:-2]+'11'+ep+'1')
            for variant in ['clean']+(['late-sbr-crc']if count==7 and base['rate']==48000 else []):
                name=base['name']+f'-f{count}'+(''if variant=='clean'else '-'+variant);rows=[];packets=[]
                for index in range(3):
                    children=[unit]*count
                    if variant!='clean'and index==1:
                        damaged=parts.copy();at=next(i for i,p in enumerate(base['frames'][0]['esc'])if p['category']==9)
                        damaged[at]=str(1-int(damaged[at][0]))+damaged[at][1:];children[-1]=packed(''.join(damaged))
                    packet,choice=protect_multiple(children,sets,0,index)
                    rows.append(dict(offset=len(blob),bytes=len(packet),choice=choice));blob.extend(packet);packets.append(packet)
                case=dict(name=name,n=base['n'],rate=base['rate'],channels=base['channels'],configuration=base['configuration'],flags=0,asc=asc.hex(),plain_asc=base['asc'],frames=rows,plain_unit_offset=unit_at,plain_unit_bytes=len(unit),source_frames=count,container_rate=base['rate'],container_frame_samples=base['container_frame_samples']*count,samples=base['container_frame_samples']*count*3,pcm_offset=0,slots=base['slots'],bands=base['bands'],video=name+'.mp4',matroska=name+'.mka')
                movie(case,packets)
                if variant=='clean':files[name+'.asc']=asc;multiple.append(case)
                else:case.update(good=base['name']+f'-f{count}',error='low-delay SBR CRC mismatch',bad_index=1);bad.append(case)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain);files['instances.bin']=bytes(information)
    files['manifest.json']=(json.dumps(dict(cases=cases,concatenated=multiple,malformed=bad,provenance='Own ELD/LD-SBR category assignment and independent scalar/CLDFB PCM; independent EP CRC/SRCPC/headers. Header-switch changes amplitude resolution for two-envelope frames. Concatenation repeats valid plain AUs as transport composition controls. No foreign codec, private media, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
