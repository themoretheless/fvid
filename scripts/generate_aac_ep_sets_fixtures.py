"""Own MPEG-4 EP predefined-set switching, short headers and playback controls."""
import json
from pathlib import Path
from generate_aac_ep_playback_fixtures import protected_header,crc
from generate_aac_ep_srcpc_fixtures import code
from generate_aac_ep_rs_fixtures import encode
from generate_aac_eld_fixtures import generate as eld,mkv
from generate_aac_sbr_dsp_fixtures import header as sbr_header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-sets'
COUNTS=[2,3,4,5,8,9,16,17,32,33,64,65,128,129,255]
def header(message):
    return message*3 if len(message)<3 else protected_header(message,True)
def attributes(index):
    return [dict(fec=0,rate=24,crc=16,length=0,dynamic=False),
            dict(fec=1,rate=2,crc=24,length=13,dynamic=False),
            dict(fec=0,rate=None,crc=None,length=15,dynamic=True),
            dict(fec=1,rate=4,crc=32,length=8,dynamic=False)][index%4]
def config(count,interleave=0,switch=0,offset=0):
    result=field(count,8)+field(interleave,2)+'001'+'001'
    for i in range(count):
        c=attributes(i+offset)
        result+=field(1,6)+'1'+field(c['dynamic'],1)*2+field(c['fec'],2)
        if c['fec']==0:result+='1'
        if interleave==2:result+=field(1 if c['fec']and switch==2 else switch,2)
        result+='0'+field(c['length'],4)
        if not c['dynamic']:result+=field(c['rate'],7 if c['fec']else 5)+field(17 if c['crc']==24 else 18 if c['crc']==32 else c['crc'],5)
        # Both forms of a one-class output permutation are exercised.
        result+=('1'+'000000')if i%2 else '0'
    return result+'1'+field(24,5)+field(16,5)
def generate():
    files={};cases=[];malformed=[];blob=bytearray()
    vectors=[dict(information=field(v,n),encoded=header(field(v,n)))for n in range(1,9)for v in range(1<<n)]
    core=eld(True,[(n,24000,c,'base',0,0,False)for n in [480,512]for c in [1,2]])
    ldroot=ROOT/'tests/fixtures/playback-errors/aac-eld-sbr';lm=json.loads((ldroot/'manifest.json').read_text());lp=(ldroot/'packets.bin').read_bytes()
    for ld in [False,True]:
        for base in json.loads(core['manifest.json'])['cases']:
            n=base['n'];channels=base['channels'];original=next(c for c in lm['cases']if c['name']==f'n{n}-c{channels}-x2-crc1')if ld else base
            original_blob=lp if ld else core['packets.bin'];rate=48000 if ld else 24000;frame_samples=n*(2 if ld else 1)
            golden=f'n{n}-c{channels}-ld{int(ld)}.f32le';files[golden]=(ldroot/(original['name']+'.f32le')).read_bytes()if ld else core[golden.replace(f'-ld{int(ld)}','')]
            prefix=field(31,5)+field(7,6)+frequency(24000)+field(channels,4)+field(n==480,1)+'000'+field(ld,1)+('11'+sbr_header(0,True)if ld else '')+'0000'+'10'
            modes=[(count,'clean')for count in COUNTS]+[(count,'corrected')for count in [3,255]]+[(3,'bad-selector'),(3,'bad-crc'),(255,'truncated-selector')]
            for count,mode in modes:
                name=f'n{n}-c{channels}-ld{int(ld)}-sets{count}-{mode}';asc=packed(prefix+config(count));files[name+'.asc']=asc;frames=[];packets=[];width=(count-1).bit_length()
                selections=[0,count-1,1,count//2,max(count-2,0),2,3,0,1,count-1,2,1]
                for i,row in enumerate(original['frames']):
                    choice=selections[i]%count;c=attributes(choice);a=row['offset'];raw=original_blob[a:a+row['bytes']];message=''.join(f'{v:08b}'for v in raw)
                    actual_rate=[0,3,4,6,8,12,16,24][i%8]if c['dynamic']else c['rate'];crc_width=[0,6,8,10,12,14,16,32][i%8]if c['dynamic']else c['crc'];parity=crc(message,crc_width)
                    if mode=='bad-crc'and i==3:
                        assert crc_width>0
                        parity=('1'if parity[0]=='0'else '0')+parity[1:]
                    coded=''.join(f'{v:08b}'for v in encode(raw+packed(parity),actual_rate))if c['fec']else code(message+parity,actual_rate,True)
                    selected=count if mode=='bad-selector'and i==3 else choice
                    selector=header(field(selected,width))
                    make=lambda stuff:(field(len(message),c['length'])if c['length']else '')+(field(i%8,3)*2 if c['dynamic']else '')+field(stuff,3)
                    h=header(make(0));stuff=(-len(selector)-len(h)-len(coded))%8;h=header(make(stuff))
                    if mode=='corrected':
                        # One error in each independently protected header.
                        pos=i%len(selector);selector=selector[:pos]+str(1-int(selector[pos]))+selector[pos+1:]
                        pos=(i*3)%len(h);h=h[:pos]+str(1-int(h[pos]))+h[pos+1:]
                    packet=packed(selector+h+coded+'0'*stuff)
                    if mode=='truncated-selector'and i==3:packet=packet[:1]
                    frames.append(dict(offset=len(blob),bytes=len(packet),choice=choice));blob.extend(packet);packets.append(packet)
                case=dict(name=name,n=n,slots=n//32,bands=64 if ld else 32,channels=channels,rate=rate,asc=asc.hex(),frames=frames,container_rate=rate,container_frame_samples=frame_samples,samples=frame_samples*12,pcm_offset=0,golden=golden,sets=count,mask=4 if channels==1 else 3)
                filename=name+'.mp4';files[filename]=video_fixture([case],blob,channels,filename=filename,write=False)['data'];case['video']=filename;files[name+'.mka']=mkv(asc,packets,rate,frame_samples,channels);case['matroska']=name+'.mka'
                if mode in ['bad-selector','bad-crc','truncated-selector']:
                    case['error']={'bad-selector':'EP predefined set index out of range','bad-crc':'EP class CRC mismatch','truncated-selector':'truncated EP BCH/Golay header'}[mode]
                    case['good']=name.replace(mode,'clean');malformed.append(case)
                else:cases.append(case)
    files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,header_vectors=vectors,provenance='Own nonzero ELD/LD-SBR packets/PCM, independent repetition/polynomial header writer, SRCPC convolution and RS long division. No private media, external codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
