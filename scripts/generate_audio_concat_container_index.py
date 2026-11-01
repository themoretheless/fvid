"""Authored FLAC/Opus containers with matching neutral VP9/FLAC video control."""
from pathlib import Path
import struct,json,hashlib
from generate_loudnorm_budget_mix_concat_regressions import build_matroska_flac_mka,build_raw_flac_48k,flac_frame,u,simple_block
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-concat-container-index'
def generate():
    files={};cases=[]
    def case(name,data,pcm,channels,frames,work,codec,exact=False):
        files[name]=data;files[name+'.f32le']=pcm;cases.append(dict(file=name,oracle=name+'.f32le',channels=channels,frames=frames,work=work,codec=codec,exact=exact))
    case('flac.mka',build_matroska_flac_mka(),b''.join(struct.pack('<f',v/32768)for v in [4096]*1920+[-4096]*1920),1,3840,2,'flac',True)
    head=b'OpusHead'+struct.pack('<BBHIhB',1,1,0,48000,0,0)
    header=ebml('1a45dfa3',ebml('4282',b'matroska'));info=ebml('1549a966',u('2ad7b1',1_000_000))
    audio_settings=ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',1))
    track=ebml('ae',u('d7',1)+u('73c5',1)+u('83',2)+ebml('86',b'A_OPUS')+ebml('63a2',head)+audio_settings)
    blocks=u('e7',0)+b''.join(simple_block(1,i*20,0x80,bytes.fromhex('f8fffe000000'))for i in range(2))
    case('opus.mka',header+ebml('18538067',info+ebml('1654ae6b',track)+ebml('1f43b675',blocks)),bytes(1920*4),1,1920,2,'opus')
    video=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+u('23e383',40_000_000)+ebml('e0',u('b0',16)+u('ba',16)))
    private=b'fLaC\x80\0\0\x22'+build_raw_flac_48k()[8:42]
    audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_FLAC')+ebml('63a2',private)+audio_settings)
    blocks=u('e7',0)
    for i in range(2):blocks+=simple_block(1,i*40,0x80,frame(8,'420',16,16))+simple_block(2,i*40,0x80,flac_frame(i,1920,False,4096 if i==0 else -4096,block_code=7,rate_code=10))
    files['video-control.mkv']=header+ebml('18538067',info+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
    files['manifest.json']=(json.dumps(dict(provenance='Authored constant FLAC and silent Opus in short Matroska containers,plus80ms neutral VP9 with the identical FLAC packets and STREAMINFO. No private media,encoder,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
