"""Own in-band RS correction-strength changes for ELD/LD-SBR; offline only."""
import json
from pathlib import Path
from generate_aac_ep_rs_fixtures import encode,corrupt
from generate_aac_ep_playback_fixtures import protected_header,crc
from generate_aac_eld_fixtures import generate as eld,mkv
from generate_aac_sbr_dsp_fixtures import header as sbr_header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-rs-escaped'
def config(k,mode):
    length=13 if mode in ['declared','truncated']else 0
    dynamic=mode=='dynamic-crc'
    # FEC=1 with escaped rate: no static k and no SRCPC termination flag.
    ep=field(1,8)+'00'+'001'+'001'+field(1,6)+'11'+field(dynamic,1)+'01'+'0'+field(length,4)
    if not dynamic:ep+=field(17 if mode=='crc24'else 16,5)
    return ep+'00',length,dynamic

def generate():
    files={};cases=[];malformed=[];blob=bytearray()
    core=eld(True,[(n,24000,c,'base',0,0,False)for n in [480,512]for c in [1,2]])
    core_cases=json.loads(core['manifest.json'])['cases'];ldroot=ROOT/'tests/fixtures/playback-errors/aac-eld-sbr'
    ldmanifest=json.loads((ldroot/'manifest.json').read_text());ldpackets=(ldroot/'packets.bin').read_bytes()
    selected=[0,1,2,4,8,16,32,64,127]
    for ld in [False,True]:
        for base in core_cases:
            n=base['n'];channels=base['channels'];original=next(c for c in ldmanifest['cases']if c['name']==f'n{n}-c{channels}-x2-crc1')if ld else base
            original_blob=ldpackets if ld else core['packets.bin'];rate=48000 if ld else 24000;frame_samples=n*(2 if ld else 1)
            golden=f'n{n}-c{channels}-ld{int(ld)}.f32le';files[golden]=(ldroot/(original['name']+'.f32le')).read_bytes()if ld else core[original['name']+'.f32le']
            prefix=field(31,5)+field(7,6)+frequency(24000)+field(channels,4)+field(n==480,1)+'000'+field(ld,1)+('11'+sbr_header(0,True)if ld else '')+'0000'+'10'
            modes=[(0,m)for m in ['until','declared','corrected','dynamic-crc','bad-crc','beyond-capability','truncated']]
            for descriptor,mode in modes:
                k=descriptor
                name=f'n{n}-c{channels}-ld{int(ld)}-k{k}-{mode}';ep,length,dynamic=config(k,mode);asc=packed(prefix+ep);files[name+'.asc']=asc
                frames=[];packets=[]
                for i,row in enumerate(original['frames']):
                    a=row['offset'];raw=original_blob[a:a+row['bytes']];rate_code=i%8;k=[0,3,4,6,8,12,16,24][rate_code]
                    crc_code=[0,2,6,7][i%4]if dynamic else 6;width=24 if mode=='crc24'else [0,6,8,10,12,14,16,32][crc_code]
                    message=''.join(f'{v:08b}'for v in raw);parity=crc(message,width)
                    if mode=='bad-crc'and i==3:parity=('1'if parity[0]=='0'else '0')+parity[1:]
                    information=raw+packed(parity);coded=encode(information,k)
                    if mode=='corrected':coded,_=corrupt(coded,len(information),k)
                    if mode=='beyond-capability'and i==3:
                        damaged=bytearray(coded);
                        for j in range(k+1):damaged[j]^=(j+1)
                        coded=bytes(damaged)
                    make=lambda stuff:(field(len(message),length)if length else '')+field(rate_code,3)+(field(crc_code,3)if dynamic else '')+field(stuff,3)
                    stub=protected_header(make(0),False);stuff=(-len(stub))%8;h=protected_header(make(stuff),False)
                    if mode=='corrected':h=('1'if h[0]=='0'else '0')+h[1:]
                    bits=h+''.join(f'{v:08b}'for v in coded)+'0'*stuff;assert len(bits)%8==0
                    packet=packed(bits)
                    if mode=='truncated'and i==3:packet=packet[:-1]
                    frames.append(dict(offset=len(blob),bytes=len(packet),rate_code=rate_code,k=k));blob.extend(packet);packets.append(packet)
                case=dict(name=name,slots=n//32,bands=64 if ld else 32,channels=channels,rate=rate,asc=asc.hex(),frames=frames,container_rate=rate,container_frame_samples=frame_samples,samples=frame_samples*12,pcm_offset=0,golden=golden,n=n,mask=4 if channels==1 else 3,rate_escape=True)
                filename=name+'.mp4';files[filename]=video_fixture([case],blob,channels,filename=filename,write=False)['data'];case['video']=filename
                files[name+'.mka']=mkv(asc,packets,rate,frame_samples,channels);case['matroska']=name+'.mka'
                if mode in ['bad-crc','beyond-capability','truncated']:
                    case['error']={'bad-crc':'EP class CRC mismatch','beyond-capability':'uncorrectable EP RS','truncated':'EP RS class/packet length mismatch'}[mode];malformed.append(case)
                else:cases.append(case)
    groups=[]
    for rate_code,k in enumerate([0,3,4,6,8,12,16,24]):
        data=bytes([0x29,0xb3]);coded=encode(data,k);damaged,_=corrupt(coded,len(data),k)
        make=lambda other,stuff:field(rate_code,3)+field(other,3)+field(stuff,3)
        stuff=(-len(protected_header(make(rate_code,0),False)))%8
        packet=lambda other,body:packed(protected_header(make(other,stuff),False)+''.join(f'{v:08b}'for v in body)+'0'*stuff)
        groups.append(dict(code=rate_code,k=k,data=data.hex(),packet=packet(rate_code,coded).hex(),corrected=packet(rate_code,damaged).hex(),mismatched=packet((rate_code+1)%8,coded).hex()))
    files['groups.json']=(json.dumps(groups,indent=2)+'\n').encode()
    files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own independent polynomial RS class writer and own ELD/LD-SBR PCM; no private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
