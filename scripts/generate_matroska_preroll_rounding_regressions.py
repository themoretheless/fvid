"""Authored short PCM video preserving audible samples at quantized zero origins."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_matroska_enabled_track_regressions import video
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-preroll-rounding'
def clip(rate,delay,origin,skip,opus=False):
    frames=rate//50;values=[(i*37)%32768-16384 for i in range(frames)]
    for index in range(min(skip,frames)):values[index]=0
    scale=1000 if origin < -32768 else 1
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',scale.to_bytes(max(1,(scale.bit_length()+7)//8),'big'))+ebml('4489',struct.pack('>d',max(20_000_000,origin+20_000_000)/scale)))
    audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_OPUS' if opus else b'A_PCM/INT/LIT')+ebml('23e383',(20_000_000).to_bytes(4,'big'))+ebml('56aa',delay.to_bytes(8,'big'))+(ebml('63a2',b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',0,48000,0,0)) if opus else b'')+ebml('e1',ebml('b5',struct.pack('>d',float(rate)))+ebml('9f',b'\1')+ebml('6264',b'\x10')))
    video_cluster=ebml('1f43b675',ebml('e7',b'\0')+ebml('a3',b'\x81\0\0\x80'+frame(8,'420',16,16)))
    stamp=max(0,origin)//scale;relative=min(0,origin)//scale
    block=b'\x82'+struct.pack('>h',relative)+b'\0'+(bytes.fromhex('f8fffe') if opus else b''.join(struct.pack('<h',v) for v in values))
    audio_cluster=ebml('1f43b675',ebml('e7',stamp.to_bytes(max(1,(stamp.bit_length()+7)//8),'big'))+ebml('a0',ebml('a1',block)+ebml('9b',(20_000_000//scale).to_bytes(4,'big'))))
    return header+ebml('18538067',info+ebml('1654ae6b',video(1,16,b'\1')+audio)+video_cluster+audio_cluster),b''.join(struct.pack('<f',v/32768.) for v in values)
def generate():
    cases=[];files={}
    for name,rate,delay,origin,skip in [
        ('delay-48000-two',48000,41667,0,2),('delay-44100-one',44100,22676,0,1),('delay-44100-four',44100,90703,0,4),
        ('control-48000-one',48000,20833,0,1),('same-sample-origin',44100,22676,22675,0),
        ('same-sample-origin-48000',48000,20834,20833,0),
        ('negative-sample-grid',44100,0,-22676,1),('control-negative-grid',48000,0,-20833,1),
        ('negative-sample-grid-48000',48000,0,-20834,1),
        ('strict-negative-fraction',48000,0,-1000,1),('strict-negative-millisecond',44100,0,-1_000_000,45),
        ('positive-origin',44100,22676,20_000_000,0),('subsample-delay',8000,1,0,0),('all-preroll',8000,100_000_000,0,160)]:
        media,raw=clip(rate,delay,origin,skip);files[name+'.mkv']=media;files[name+'-source.f32le']=raw;files[name+'-expected.f32le']=raw[skip*4:]
        cases.append({'name':name,'file':name+'.mkv','rate':rate,'delay_ns':delay,'origin_ns':origin,'skip':skip,'pts_ns':max(0,origin-delay+skip*1_000_000_000//rate)})
    from generate_matroska_padding_rounding_regressions import vorbis_clip
    files['opus-delay-two.mkv']=clip(48000,41667,0,2,True)[0]
    files['vorbis-delay-two.mkv']=vorbis_clip(0,delay_ns=41667)
    files['overflow.mkv']=clip(8000,(1<<64)-1,0,0)[0]
    files['manifest.json']=(json.dumps({'provenance':'Authored20ms VP9/PCM with known sample-grid CodecDelay/origins quantized to integer ns, plus strict off-grid negative controls. All original timestamp fields remain explicit. No encoder/private media/network.','cases':cases,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
