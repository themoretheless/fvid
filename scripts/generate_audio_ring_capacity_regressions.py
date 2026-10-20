"""Authored700ms neutral video and one682ms48kHz PCM access unit."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_matroska_enabled_track_regressions import video
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-ring-capacity'
def clip(frames=32768,sample_rate=48000,video_frames=35,packets=1,origin=0,packet_times=None,bits=16,floating=False,channels=1,payload_transform=None):
    values=[(i*37)%32768-16384 for i in range(frames)]
    values=[v if channel%2==0 else -v for v in values for channel in range(channels)]
    pcm=b''.join(struct.pack('<f',v/32768.) if floating else struct.pack('<h',v) for v in values)
    if payload_transform is not None:pcm=payload_transform(pcm)
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',float(video_frames*20))))
    audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_PCM/FLOAT/IEEE' if floating else b'A_PCM/INT/LIT')+ebml('23e383',(frames*1_000_000_000//sample_rate).to_bytes(8,'big'))+ebml('e1',ebml('b5',struct.pack('>d',float(sample_rate)))+ebml('9f',channels.to_bytes(max(1,(channels.bit_length()+7)//8),'big'))+ebml('6264',bits.to_bytes(max(1,(bits.bit_length()+7)//8),'big'))))
    blocks=ebml('e7',origin.to_bytes(max(1,(origin.bit_length()+7)//8),'big'))
    for packet in range(packets):blocks+=ebml('a3',b'\x82'+struct.pack('>h',packet_times[packet] if packet_times is not None else round(packet*frames*1000/sample_rate))+b'\x80'+pcm)
    for index in range(video_frames):blocks+=ebml('a3',b'\x81'+struct.pack('>h',index*20)+b'\x80'+frame(8,'420',16,16))
    return header+ebml('18538067',info+ebml('1654ae6b',video(1,16,b'\1')+audio)+ebml('1f43b675',blocks)),b''.join(struct.pack('<f',v/32768.) for v in values)
def generate():
    video,pcm=clip()
    files={'large-pcm.mkv':video,'expected.f32le':pcm}
    files['manifest.json']=(json.dumps({'provenance':'Authored700ms16-square VP9 and32768-sample48kHz mono16bit PCM access unit (682ms). No encoder/private media/network.','frames':32768,'video_frames':35,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
