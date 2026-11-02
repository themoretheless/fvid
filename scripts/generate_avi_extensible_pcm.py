"""Invent WAVE_FORMAT_EXTENSIBLE AVI/RIFF pairs and exact PCM; no encoder/network."""
from pathlib import Path
import json,struct
from generate_avi_audio_eof_regressions import chunk,listing,stream
from generate_mp4_seek_io_regressions import generate as video
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-extensible-pcm'
def make(fmt,raw,channels,bits,n,parts,start=0,unsupported_first=False):
    coded=frame(8,'420',16,16);stride=channels*(bits//8)
    ah=bytearray(stream(b'auds',bytes(4),1,8000,n,max(parts)*stride));struct.pack_into('<I',ah,28,start);struct.pack_into('<I',ah,44,stride)
    audio=listing(b'strl',chunk(b'strh',ah)+chunk(b'strf',fmt));vf=struct.pack('<IiiHH4sIiiII',40,16,16,1,24,b'VP90',len(coded),0,0,0,0)
    v=listing(b'strl',chunk(b'strh',stream(b'vids',b'VP90',1,50,1,len(coded)))+chunk(b'strf',vf));main=struct.pack('<14I',20000,0,0,0,1,0,2,max(len(coded),max(parts)*stride),16,16,0,0,0,0)
    records=chunk(b'00dc',coded);at=0
    for count in parts:records+=chunk(b'02wb'if unsupported_first else b'01wb',raw[at*stride:(at+count)*stride]);at+=count
    assert at==n
    if unsupported_first:
        unknown=bytearray(fmt);unknown[39]^=1
        audio=listing(b'strl',chunk(b'strh',ah)+chunk(b'strf',unknown))+audio
        main=bytearray(main);struct.pack_into('<I',main,24,3)
        records+=b''.join(chunk(b'01wb',raw[sum(parts[:j])*stride:sum(parts[:j+1])*stride])for j in range(len(parts)))
    body=b'AVI '+listing(b'hdrl',chunk(b'avih',main)+v+audio)+listing(b'movi',records)
    return b'RIFF'+struct.pack('<I',len(body))+body
def generate():
    v=video();files={'neutral-video.mp4':v['three-colors.mp4'],**{f'frame-{i}.yuv16':v[f'frame-{i}.yuv16']for i in range(3)}};cases=[];bad=[]
    profiles=[(tag,bits,bits,c,mask,4099,[3,2048,2048],0)for tag,bits in [(1,8),(1,16),(1,24),(1,32),(3,32),(3,64)]for c,mask in [(1,4),(2,48),(6,63)]]+[(1,24,20,2,3,4099,[4099],0),(1,32,24,6,63,4099,[7,4092],73),(3,32,32,9,0,4099,[4099],0)]
    pattern=[-.75,-.5,-.25,0.,.25,.5,.75,.125,-.125]
    for i,(tag,bits,valid,channels,mask,n,parts,start)in enumerate(profiles):
        raw=b'';gold=b''
        for t in range(n):
            for c in range(channels):
                x=pattern[(t+c*2)%len(pattern)]/(2**(c%3))
                raw+=struct.pack('<f'if bits==32 else'<d',x)if tag==3 else bytes([int(x*128)+128])if bits==8 else int(x*(1<<(bits-1))).to_bytes(bits//8,'little',signed=True)
                gold+=struct.pack('<f',x)
        stride=channels*(bits//8);fmt=struct.pack('<HHIIHHHHII',65534,channels,8000,8000*stride,stride,bits,22,valid,mask,tag)+bytes.fromhex('00001000800000aa00389b71');name=f'p{i}-{tag}-{bits}-v{valid}-c{channels}'
        files[name+'.avi']=make(fmt,raw,channels,bits,n,parts,start);wave=b'WAVE'+chunk(b'fmt ',fmt)+chunk(b'data',raw);files[name+'.wav']=b'RIFF'+struct.pack('<I',len(wave))+wave;files[name+'.f32le']=gold
        cases.append(dict(name=name,tag=tag,bits=bits,valid=valid,channels=channels,mask=mask,frames=n,parts=parts,start=start))
        if i==4:
            mixed=name+'-after-unsupported';files[mixed+'.avi']=make(fmt,raw,channels,bits,n,parts,start,True);files[mixed+'.wav']=files[name+'.wav'];files[mixed+'.f32le']=gold;case=cases[-1].copy();case['name']=mixed;case['unsupported_first']=True;cases.append(case)
        if i==4:
            for kind,at,value in [('cbsize',16,23),('valid',18,0),('mask',20,7)]:
                altered=bytearray(fmt);struct.pack_into('<I'if kind=='mask'else'<H',altered,at,value);fn='bad-'+kind+'.avi';files[fn]=make(bytes(altered),raw,channels,bits,n,parts);bad.append(dict(file=fn,kind='invalid',needle={'cbsize':'extensible fmt','valid':'valid-bit','mask':'channel mask'}[kind]))
            altered=bytearray(fmt);altered[39]^=1;fn='unsupported-guid.avi';files[fn]=make(bytes(altered),raw,channels,bits,n,parts);bad.append(dict(file=fn,kind='unsupported'))
    files['manifest.json']=(json.dumps(dict(provenance='Authored RIFF/AVI and WAVE extensible headers,dyadic PCM,neutral authored VP9. No private source media,encoder or network.',cases=cases,malformed=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
