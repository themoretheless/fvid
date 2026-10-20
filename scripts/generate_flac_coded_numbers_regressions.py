"""Author short FLAC multi-byte coded numbers with independent constant PCM."""
import json,hashlib,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-coded-numbers'
def crc(data,width,poly):
    c=0
    for value in data:
        c^=value<<(width-8)
        for _ in range(8):c=((c<<1)^(poly if c&(1<<(width-1)) else 0))&((1<<width)-1)
    return c
def number(value):
    if value<128:return bytes([value])
    for length in range(2,8):
        if value<(1<<(5*length+1)):
            payload=[0x80|((value>>(6*i))&63) for i in range(length-2,-1,-1)]
            return bytes([((0xff<<(8-length))&255)|(value>>(6*(length-1))),*payload])
    raise ValueError(value)
def frame(value,samples=64,variable=False,level=4096,raw=None,block_code=6,rate_code=0,rate_extra=b''):
    header=bytes([255,249 if variable else 248,(block_code<<4)|rate_code,8])+(number(value) if raw is None else raw)
    if block_code==6:header+=bytes([samples-1])
    elif block_code==7:header+=struct.pack('>H',samples-1)
    header+=rate_extra;header+=bytes([crc(header,8,7)]);data=header+b'\0'+struct.pack('>h',level);return data+struct.pack('>H',crc(data,16,0x8005))
def stream(packets,sizes,rate=44100):
    info=struct.pack('>HH',min(sizes),max(sizes))+bytes(6)+((rate<<44)|(15<<36)|sum(sizes)).to_bytes(8,'big')+bytes(16)
    return b'fLaC\x80\0\0\x22'+info+b''.join(packets)
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[];headers=[];invalid=[]
    for label,count,size,variable in [('fixed-128',128,64,False),('fixed-129',129,64,False),('fixed-130',130,64,False),('variable-2-small',2,64,True),('variable-3-small',3,64,True),('variable-2-wide',2,128,True),('fixed-3',3,128,False)]:
        packets=[];specs=[]
        for i in range(count):
            level=[4096,-4096,8192][i%3];packet=frame(i*size if variable else i,size,variable,level);filename=f'{label}-{i}.frame';files[filename]=packet;packets.append(packet);specs.append(dict(file=filename,samples=size,level=level))
        files[label+'.flac']=stream(packets,[size]*count);cases.append(dict(file=label+'.flac',packets=specs))
    for value in [127,128,2047,2048,65535,65536,2097151,2097152,67108863,67108864,2147483647,2147483648,68719476735]:
        name=f'header-{value}.frame';files[name]=frame(value,variable=True);headers.append(name)
    for label,raw,variable in [('continuation',b'\xc2\x40',True),('overlong',b'\xc0\x80',True),('leading',b'\x80',True),('truncated',b'\xc2',True),('eight-byte',b'\xff'+b'\x80'*7,True),('fixed-seven-byte',number(2147483648),False)]:
        name='invalid-'+label+'.frame';files[name]=frame(0,variable=variable,raw=raw);invalid.append(name)
    files['manifest.json']=(json.dumps(dict(provenance='Seven short authored constant16bitFLAC streams exercise frame/sample coded numbers at127/128. Independent PCMlevels andnativepacket decode distinguish indexing failure fromcodec errors. Canonical2..7byteheader-only numeric probes are not complete longaudiofiles;malformed coded-number guards require refusal. NativeVP9/PCMcompanion,no private media/encoder/network.',cases=cases,headers=headers,invalid=invalid,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
