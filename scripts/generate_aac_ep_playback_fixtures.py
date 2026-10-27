"""Own EP2 frame routing/CRC/FEC playback matrix. Explicit offline generation."""
import json
from pathlib import Path
from generate_aac_ep_srcpc_fixtures import code,POWERS
from generate_aac_eld_fixtures import generate as eld,mkv
from generate_aac_sbr_dsp_fixtures import header as sbr_header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-playback'

def remainder(message,exponents):
    degree=max(exponents);value=int(message or '0',2)<<degree;polynomial=sum(1<<v for v in exponents)
    while value.bit_length()>degree:value^=polynomial<<(value.bit_length()-degree-1)
    return f'{value:0{degree}b}'
def crc(message,width=16):return ''.join('1'if b=='0'else '0'for b in remainder(message,POWERS[width]))if width else ''
def protected_header(message,extended):
    n=len(message)
    if n<=16:
        powers=([3,1,0]if n<=4 else[8,7,6,4,0]if n<=7 else[11,9,7,6,5,1,0]if n<=12 else[15,11,10,9,8,7,5,3,2,1,0])
        return message+remainder(message,powers)
    return code(message+(crc(message)if extended else ''.join('1'if b=='0'else '0'for b in remainder(message,[4,3,2,0]))),24 if extended else 8,True)
def ep_config(rate,mode):
    length={'length':8,'bch31':13,'large':15,'extended':15,'truncated':8}.get(mode,0)
    rate_dynamic=mode in ['rate','dual','large','extended'];crc_dynamic=mode in ['crc','dual','large','extended'];extended=mode=='extended'
    bits=field(1,8)+'00'+'001'+'001'+field(1,6)+'1'+field(rate_dynamic,1)+field(crc_dynamic,1)+'00'+'1'+'0'+field(length,4)
    if not rate_dynamic:bits+=field(rate,5)
    if not crc_dynamic:bits+=field(int(mode.split('-')[1])if mode.startswith('width-')else 16,5)
    bits+='0'+field(extended,1)
    if extended:bits+=field(24,5)+field(16,5)
    return bits,length,rate_dynamic,crc_dynamic,extended

def generate():
    files={};cases=[];malformed=[];blob=bytearray();header_vectors=[]
    for n in range(3,25):
        for extended in [False,True]:
            for pattern in range(3):
                bits=''.join(str(0 if pattern==0 else 1 if pattern==1 else (i//2+i//5)%2)for i in range(n))
                header_vectors.append(dict(information=bits,extended=extended,encoded=protected_header(bits,extended)))
    core=eld(True,[(n,24000,c,'base',0,0,False)for n in [480,512]for c in [1,2]])
    core_cases=json.loads(core['manifest.json'])['cases']
    ldroot=ROOT/'tests/fixtures/playback-errors/aac-eld-sbr';ld_manifest=json.loads((ldroot/'manifest.json').read_text());ld_packets=(ldroot/'packets.bin').read_bytes()
    for ld in [False,True]:
        for base in core_cases:
            n=base['n'];channels=base['channels'];original=next(c for c in ld_manifest['cases']if c['name']==f'n{n}-c{channels}-x2-crc1')if ld else base
            original_blob=ld_packets if ld else core['packets.bin'];rate=48000 if ld else 24000;frame_samples=n*(2 if ld else 1)
            golden=f'n{n}-c{channels}-ld{int(ld)}.f32le'
            files[golden]=(ldroot/(original['name']+'.f32le')).read_bytes()if ld else core[original['name']+'.f32le']
            prefix=field(31,5)+field(7,6)+frequency(24000)+field(channels,4)+field(n==480,1)+'000'+field(ld,1)+('11'+sbr_header(0,True)if ld else '')+'0000'+'10'
            modes=[(r,'stuffing')for r in range(25)]+[(24,m)for m in ['length','bch31','rate','crc','dual','large','extended']]
            if n==512 and channels==1 and not ld:modes += [(24,'corrected'),(24,'bad-crc'),(24,'truncated')]+[(24,f'width-{code}')for code in range(19)]
            for r,mode in modes:
                name=f'n{n}-c{channels}-ld{int(ld)}-r{r}-{mode}'
                ep,length,rate_dynamic,crc_dynamic,extended=ep_config(r,mode);asc=packed(prefix+ep);files[name+'.asc']=asc
                frames=[];packets=[]
                for i,row in enumerate(original['frames']):
                    a=row['offset'];raw=original_blob[a:a+row['bytes']];message=''.join(f'{v:08b}'for v in raw)
                    actual_index=i%8 if rate_dynamic else 7
                    actual_rate=[0,3,4,6,8,12,16,24][actual_index]if rate_dynamic else r
                    crc_code=0 if crc_dynamic and i%2==0 else 6
                    width=[0,6,8,10,12,14,16,32][crc_code]if crc_dynamic else list(POWERS)[int(mode.split('-')[1])]if mode.startswith('width-')else 16
                    parity=crc(message,width)
                    if mode=='bad-crc'and i==3:parity=('1'if parity[0]=='0'else '0')+parity[1:]
                    coded=code(message+parity,actual_rate,True)
                    make=lambda stuff:(field(len(message),length)if length else '')+(field(actual_index,3)if rate_dynamic else '')+(field(crc_code,3)if crc_dynamic else '')+field(stuff,3)
                    stub=protected_header(make(0),extended);stuff=(-len(stub)-len(coded))%8
                    h=protected_header(make(stuff),extended)
                    if mode=='corrected':
                        h=('1'if h[0]=='0'else '0')+h[1:]
                        coded=coded[:7]+('1'if coded[7]=='0'else '0')+coded[8:]
                    bits=h+coded+'0'*stuff;assert len(bits)%8==0
                    packet=packed(bits)
                    if mode=='truncated'and i==3:packet=packet[:-1]
                    frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
                case=dict(name=name,slots=n//32,bands=64 if ld else 32,channels=channels,rate=rate,asc=asc.hex(),frames=frames,container_rate=rate,container_frame_samples=frame_samples,samples=frame_samples*12,pcm_offset=0,golden=golden,n=n,mask=4 if channels==1 else 3)
                filename=name+'.mp4';files[filename]=video_fixture([case],blob,channels,filename=filename,write=False)['data'];case['video']=filename
                files[name+'.mka']=mkv(asc,packets,rate,frame_samples,channels);case['matroska']=name+'.mka'
                if mode in ['bad-crc','truncated']:case['error']='EP class CRC mismatch'if mode=='bad-crc'else 'EP class/packet length mismatch';malformed.append(case)
                else:cases.append(case)
    files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,header_vectors=header_vectors,provenance='Own ELD/LD-SBR PCM and packet sources, independent polynomial header/CRC encoding and convolution encoder. No foreign decoder, private source, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
