"""Author native sample-clock duration boundaries across FLAC/WAV/AIFF/AU."""
import json,hashlib,struct
from pathlib import Path
from generate_flac_coded_numbers_regressions import frame,stream
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/sample-duration-grid'
def chunk(tag,data,endian):return tag+struct.pack(endian+'I',len(data))+data+(b'\0' if len(data)&1 else b'')
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for rate,size,count in [(44100,128,3),(44100,64,3),(48000,64,1),(48000,128,1),(96000,64,1),(8000,128,3)]:
        frames=[level for i in range(count) for level in [[4096,-4096,8192][i%3]]*size];le=b''.join(struct.pack('<h',v) for v in frames);be=b''.join(struct.pack('>h',v) for v in frames);samples=len(frames)
        riff=b'WAVE'+chunk(b'fmt ',struct.pack('<HHIIHH',1,1,rate,rate*2,2,16),'<')+chunk(b'data',le,'<');wav=b'RIFF'+struct.pack('<I',len(riff))+riff
        exponent=rate.bit_length()-1;extended=struct.pack('>HQ',16383+exponent,rate<<(63-exponent));body=b'AIFF'+chunk(b'COMM',struct.pack('>HIH',1,samples,16)+extended,'>')+chunk(b'SSND',bytes(8)+be,'>');aiff=b'FORM'+struct.pack('>I',len(body))+body
        au=b'.snd'+struct.pack('>IIIII',24,len(be),3,rate,1)+be
        flac=stream([frame(i,size,False,[4096,-4096,8192][i%3]) for i in range(count)],[size]*count,rate)
        for kind,data in [('wav',wav),('aiff',aiff),('au',au),('flac',flac)]:
            file=f'{kind}-{rate}-{size}-{count}.{kind}';files[file]=data;cases.append(dict(file=file,kind=kind,rate=rate,samples=samples,size=size,count=count,end_ns=samples*1000000000//rate))
    files['manifest.json']=(json.dumps(dict(provenance='Twenty-four authored nativePCM/constantFLACtracks at six sample-clock boundaries. Independent exactinteger endpoint versusnearest-nanosecond floatduration;three clockcases round upward andthree healthycontrols. Short VP9/PCMcompanion,no encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
