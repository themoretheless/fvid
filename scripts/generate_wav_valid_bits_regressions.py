"""Author20ms valid-bit precision/storage-width WAVE extensible examples."""
import hashlib,json,struct
from pathlib import Path
from generate_wav_unsigned8_regressions import wav
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wav-valid-bits'
def generate():
    files={};cases=[]
    for index,(storage,valid) in enumerate([(16,12),(24,20),(32,24),(32,16),(16,8),(8,7)]):
        channels=1+index%2;frames=160;half=1<<(valid-1);values=[-half,-1,0,1,half-1]
        samples=[values[(i+c)%len(values)] for i in range(frames) for c in range(channels)]
        packed=[(v+(half if storage==8 else 0))<<(storage-valid) for v in samples]
        payload=b''.join(v.to_bytes(storage//8,'little',signed=storage!=8) for v in packed)
        name=f'pcm{valid}-in-{storage}'
        files[name+'.wav']=wav(8000,channels,storage,payload,extensible=True,valid_bits=valid)
        files[name+'-control.wav']=wav(8000,channels,storage,payload)
        files[name+'.f32le']=b''.join(struct.pack('<f',v/half) for v in samples)
        files[name+'-video.mkv']=clip(frames,8000,1,packets=1,bits=storage,channels=channels,payload_transform=lambda _,data=payload:data)[0]
        cases.append({'name':name,'storage':storage,'valid':valid,'channels':channels,'frames':frames})
    files['invalid-container12.wav']=wav(8000,1,12,bytes(160),extensible=True,valid_bits=8)
    files['invalid-valid-zero.wav']=wav(8000,1,16,bytes(320),extensible=True,valid_bits=0)
    files['invalid-valid-too-wide.wav']=wav(8000,1,16,bytes(320),extensible=True,valid_bits=17)
    files['invalid-float-precision.wav']=wav(8000,1,32,bytes(640),tag=3,extensible=True,valid_bits=16)
    files['float-control.wav']=wav(8000,1,32,bytes(640),tag=3,extensible=True,valid_bits=32)
    files['manifest.json']=(json.dumps({'provenance':'Authored20ms8k PCM with left-aligned valid bits and zero padding, signed/unsigned boundary values, mono/stereo. Plain Wave and matching native VP9/PCM controls. Corrupt header precision/container variants are distinct refusals. No encoder/private media/network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
