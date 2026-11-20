"""Own mode-1/mode-2 recursive EP interleaving playback and failure sources."""
import json
from pathlib import Path
from generate_aac_ep_playback_fixtures import crc
from generate_aac_ep_srcpc_fixtures import code
from generate_aac_ep_rs_fixtures import encode
from generate_aac_eld_fixtures import generate as eld,mkv
from generate_aac_sbr_dsp_fixtures import header as sbr_header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-interleave-playback'
from generate_aac_ep_sets_fixtures import header,attributes,config
from generate_aac_ep_interleave_fixtures import stage
def interleave(x,y,width):return ''.join(stage(list(x),list(y),width))
def window(n,encoded):return 3 if n<=2 else len(encoded)if n<=16 else 28
def generate():
    files={};cases=[];malformed=[];blob=bytearray()
    core=eld(True,[(n,24000,c,'base',0,0,False)for n in [480,512]for c in [1,2]])
    ldroot=ROOT/'tests/fixtures/playback-errors/aac-eld-sbr';lm=json.loads((ldroot/'manifest.json').read_text());lp=(ldroot/'packets.bin').read_bytes()
    for ld in [False,True]:
        for base in json.loads(core['manifest.json'])['cases']:
            n=base['n'];channels=base['channels'];original=next(c for c in lm['cases']if c['name']==f'n{n}-c{channels}-x2-crc1')if ld else base
            original_blob=lp if ld else core['packets.bin'];rate=48000 if ld else 24000;frame_samples=n*(2 if ld else 1)
            golden=f'n{n}-c{channels}-ld{int(ld)}.f32le';files[golden]=(ldroot/(original['name']+'.f32le')).read_bytes()if ld else core[golden.replace(f'-ld{int(ld)}','')]
            prefix=field(31,5)+field(7,6)+frequency(24000)+field(channels,4)+field(n==480,1)+'000'+field(ld,1)+('11'+sbr_header(0,True)if ld else '')+'0000'+'10'
            profiles=[(1,0)]+[(2,s)for s in range(4)]
            counts=[1,2,3,5,9,17,33,65,129,255]
            modes=[(count,'clean',t,s,0)for count in counts for t,s in profiles]
            modes +=[(1,'clean',t,s,o)for o in [1,2,3]for t,s in profiles]
            modes +=[(3,v,t,s,0)for v in ['corrected','bad-selector','bad-crc','truncated']for t,s in profiles]
            for count,mode,kind,switch,offset in modes:
                name=f'n{n}-c{channels}-ld{int(ld)}-sets{count}-i{kind}-s{switch}-o{offset}-{mode}'
                asc=packed(prefix+config(count,kind,switch,offset));files[name+'.asc']=asc;frames=[];packets=[];width=(count-1).bit_length()
                selections=[0,count-1,1,count//2,max(count-2,0),2,3,0,1,count-1,2,1]
                for i,row in enumerate(original['frames']):
                    choice=selections[i]%count;c=attributes(choice+offset);a=row['offset'];raw=original_blob[a:a+row['bytes']];message=''.join(f'{v:08b}'for v in raw)
                    actual_rate=[0,3,4,6,8,12,16,24][i%8]if c['dynamic']else c['rate'];crc_width=[0,6,8,10,12,14,16,32][i%8]if c['dynamic']else c['crc'];parity=crc(message,crc_width)
                    if mode=='bad-crc'and i==3:
                        assert crc_width>0
                        parity=('1'if parity[0]=='0'else '0')+parity[1:]
                    coded=''.join(f'{v:08b}'for v in encode(raw+packed(parity),actual_rate))if c['fec']else code(message+parity,actual_rate,True)
                    selected=count if mode=='bad-selector'and i==3 else choice
                    selector=header(field(selected,width))if width else ''
                    make=lambda stuff:(field(len(message),c['length'])if c['length']else '')+(field(i%8,3)*2 if c['dynamic']else '')+field(stuff,3)
                    h=header(make(0));stuff=(-len(selector)-len(h)-len(coded))%8;h=header(make(stuff))
                    if mode=='corrected':
                        if selector:
                            pos=i%len(selector);selector=selector[:pos]+str(1-int(selector[pos]))+selector[pos+1:]
                        pos=(i*3)%len(h);h=h[:pos]+str(1-int(h[pos]))+h[pos+1:]
                        if c['fec']or actual_rate==24:coded=coded[:7]+str(1-int(coded[7]))+coded[8:]
                    if mode=='truncated'and i==3:coded=coded[:-8]
                    if c['fec']==0 and(kind==1 or switch==2):coded=interleave(coded,'',28)
                    # Stuffing is appended AFTER class stages, BEFORE headers.
                    payload=interleave(h,coded+'0'*stuff,window(len(make(stuff)),h))
                    if selector:payload=interleave(selector,payload,window(width,selector))
                    assert len(payload)%8==0
                    packet=packed(payload)
                    frames.append(dict(offset=len(blob),bytes=len(packet),choice=choice));blob.extend(packet);packets.append(packet)
                case=dict(name=name,n=n,slots=n//32,bands=64 if ld else 32,channels=channels,rate=rate,asc=asc.hex(),frames=frames,container_rate=rate,container_frame_samples=frame_samples,samples=frame_samples*12,pcm_offset=0,golden=golden,sets=count,interleave=kind,switch=switch,variant_offset=offset,mask=4 if channels==1 else 3)
                filename=name+'.mp4';files[filename]=video_fixture([case],blob,channels,filename=filename,write=False)['data'];case['video']=filename;files[name+'.mka']=mkv(asc,packets,rate,frame_samples,channels);case['matroska']=name+'.mka'
                if mode in ['bad-selector','bad-crc','truncated']:
                    case['error']={'bad-selector':'EP predefined set index out of range','bad-crc':'EP class CRC mismatch','truncated':'EP RS class/packet length mismatch'}[mode]
                    case['good']=name.replace(mode,'clean');malformed.append(case)
                else:cases.append(case)
    files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own ELD/LD-SBR PCM, explicit occupied-cell interleave matrix, polynomial header writer, SRCPC convolution and RS long division. No private media, external codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
