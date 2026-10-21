"""Author short WAVE/AU fixed-frame orphan-tail admission regressions."""
import hashlib,json,struct
from pathlib import Path
from generate_wav_unsigned8_regressions import wav
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/pcm-run-alignment'
def law_sample(code,alaw):
    if alaw:
        code^=0x55;exponent=(code>>4)&7;value=(code&15)<<4
        value=value+8 if exponent==0 else (value+0x108)<<(exponent-1)
        return value if code&128 else -value
    code=(~code)&255;value=(((code&15)<<3)+0x84)<<((code>>4)&7);value-=0x84
    return -value if code&128 else value

def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    layouts=[('int16',16,1,'int'),('int24-stereo',24,2,'int'),('int32',32,1,'int'),('float32-stereo',32,2,'float'),('float64',64,1,'float'),('int8-stereo',8,2,'int'),('alaw-stereo',8,2,'alaw'),('mulaw-stereo',8,2,'mulaw'),('odd-mono8',8,1,'int')]
    for container in ['wav','au']:
        for label,bits,channels,coding in layouts:
            frames=161 if label=='odd-mono8' else 160;values=[(i+c*37)%128-64 for i in range(frames) for c in range(channels)];little=container=='wav'
            if coding in ['alaw','mulaw']:
                data=bytes((i*37)%256 for i in range(frames*channels));samples=[law_sample(v,coding=='alaw')/32768. for v in data]
            elif coding=='float':data=b''.join(struct.pack(('<' if little else '>')+('f' if bits==32 else 'd'),v/128.) for v in values);samples=[v/128. for v in values]
            else:
                data=bytes(v+128 for v in values) if bits==8 and little else b''.join((v*(1<<(bits-8))).to_bytes(bits//8,'little' if little else 'big',signed=True) for v in values);samples=[v/128. for v in values]
            def wrap(payload,footer=False):
                if container=='wav':return wav(8000,channels,bits,payload,tag={'int':1,'float':3,'alaw':6,'mulaw':7}[coding])
                tag={'int':{8:2,16:3,24:4,32:5}.get(bits),'float':6 if bits==32 else 7,'alaw':27,'mulaw':1}[coding]
                return struct.pack('>6I',0x2e736e64,28,len(payload),tag,8000,channels)+b'info'+payload+(b'\x81' if footer else b'')
            name=container+'-'+label;files[name+'.'+container]=wrap(data,footer=label=='odd-mono8');files[name+'.f32le']=b''.join(struct.pack('<f',v) for v in samples)
            tails=sorted(set([1,bits//8*channels-1])) if bits//8*channels>1 else []
            bad=[]
            for tail in tails:filename=name+f'-orphan-{tail}.'+container;files[filename]=wrap(data+bytes([1])*tail);bad.append({'file':filename,'tail':tail})
            cases.append({'name':name,'kind':container,'frames':frames,'channels':channels,'file':name+'.'+container,'expected':name+'.f32le','bad':bad})
    files['manifest.json']=(json.dumps({'provenance':'Authored20ms PCM/floats/G711 WAVE/AU with complete frame controls and explicitly declared orphan audio bytes. Existing window alignment silently removes those bytes before native decode. Odd mono8 RIFF word pad and AU annotation/footer are valid controls. Matching VP9/PCM video. No private media, encoder or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
