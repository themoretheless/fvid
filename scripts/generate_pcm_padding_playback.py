"""Author valid-width and nonzero-unused-bit RIFF/AVI probes,without encoder/network."""
from pathlib import Path
import json,struct
from generate_avi_extensible_pcm import make
from generate_avi_audio_eof_regressions import chunk
from generate_mp4_seek_io_regressions import generate as video
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/pcm-padding-playback'
def generate():
    v=video();files={'neutral-video.mp4':v['three-colors.mp4'],**{f'frame-{i}.yuv16':v[f'frame-{i}.yuv16']for i in range(3)}};good=[];bad=[];n=4099
    for bits,valid,channels in [(8,7,1),(16,12,2),(24,20,1),(32,24,2),(16,1,1),(32,1,2)]:
        width=bits//8;half=1<<(valid-1);values=[-half,-1,0,half-1];padding=bits-valid
        def packed(bad_frame=None,full=False):
            raw=b''
            for t in range(n):
                for c in range(channels):
                    value=values[(t+c)%len(values)];value=(value+(half if bits==8 else 0))<<padding
                    if full:value|=1
                    if t==bad_frame and c==channels-1:value|=1<<min(padding-1,8)
                    raw+=value.to_bytes(width,'little',signed=bits!=8)
            return raw
        def write(name,raw,declared_valid):
            stride=channels*width;fmt=struct.pack('<HHIIHHHHII',65534,channels,8000,8000*stride,stride,bits,22,declared_valid,4 if channels==1 else 3,1)+bytes.fromhex('00001000800000aa00389b71');body=b'WAVE'+chunk(b'fmt ',fmt)+chunk(b'data',raw)
            files[name+'.wav']=b'RIFF'+struct.pack('<I',len(body))+body;files[name+'.avi']=make(fmt,raw,channels,bits,n,[2048,1,2050])
        key=f's{bits}-v{valid}-c{channels}'
        for full in [False,True]:
            raw=packed(full=full);name=key+('-full-width'if full else'-valid');write(name,raw,bits if full else valid)
            integers=[int.from_bytes(raw[i:i+width],'little',signed=bits!=8)-(128 if bits==8 else 0)for i in range(0,len(raw),width)]
            files[name+'.f32le']=b''.join(struct.pack('<f',x/(1<<(bits-1)))for x in integers);good.append(dict(name=name,bits=bits,valid=bits if full else valid,channels=channels,frames=n))
        for position in [0,2048,n-1]:
            name=key+f'-padding-at{position}';write(name,packed(bad_frame=position),valid);bad.append(dict(name=name,bits=bits,valid=valid,channels=channels,frame=position))
    files['manifest.json']=(json.dumps(dict(provenance='Analytic PCM at signed/unsigned limits;unused bits explicitly authored at first,window-boundary,last samples. Full-width same-low-bit controls. Handcrafted RIFF/AVI and authored neutral VP9. No private media,encoder or network.',good=good,bad=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
