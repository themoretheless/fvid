"""Authored60ms VP9/PCM with block padding spanning several laced frames."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_matroska_enabled_track_regressions import video
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-lace-padding'
def clip(mode,padding):
    values=[(i*37)%32768-16384 for i in range(480)]
    packets=[b''.join(struct.pack('<h',v) for v in values[i:i+160]) for i in range(0,480,160)]
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',60.)))
    audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_PCM/INT/LIT')+ebml('23e383',(20_000_000).to_bytes(4,'big'))+ebml('e1',ebml('b5',struct.pack('>d',8000.))+ebml('9f',b'\1')+ebml('6264',b'\x10')))
    lace={'fixed':(4,b''),'xiph':(2,bytes([255,65,255,65])),'ebml':(6,bytes([0x41,0x40,0xbf]))}[mode]
    block=b'\x82\0\0'+bytes([lace[0],2])+lace[1]+b''.join(packets)
    group=ebml('a1',block)+ebml('9b',b'\x3c')+ebml('75a2',padding.to_bytes(8,'big',signed=True))
    blocks=ebml('e7',b'\0')+ebml('a0',group)
    for i in range(3):blocks+=ebml('a3',b'\x81'+struct.pack('>h',i*20)+b'\x80'+frame(8,'420',16,16))
    return header+ebml('18538067',info+ebml('1654ae6b',video(1,16,b'\1')+audio)+ebml('1f43b675',blocks)),values

def generate():
    files={};cases=[]
    for mode in ['fixed','xiph','ebml']:
        for label,pad in [('control',0),('tail-one',10_000_000),('head-one',-10_000_000),('tail-boundary',20_000_000),('tail-span',30_000_000),('head-span',-30_000_000),('tail-rounding',30_000_001),('head-rounding',-30_000_001),('tail-whole',60_000_000),('head-whole',-60_000_000)]:
            name=mode+'-'+label;media,values=clip(mode,pad);files[name+'.mkv']=media
            trim=(abs(pad)*8000+500_000_000)//1_000_000_000
            first=trim if pad<0 else 0;last=480-trim if pad>0 else 480
            files[name+'.f32le']=b''.join(struct.pack('<f',v/32768.) for v in values[first:last])
            intervals=[]
            for index in range(3):
                start=max(first,index*160);end=min(last,(index+1)*160)
                if start<end:intervals.append({'pts_ns':start*125000,'first':start,'frames':end-start})
            cases.append({'file':name+'.mkv','oracle':name+'.f32le','padding_ns':pad,'first':first,'last':last,'packets':intervals})
    files['expected-source.f32le']=b''.join(struct.pack('<f',v/32768.) for v in clip('fixed',0)[1])
    for label,pad in [('too-large',60_000_001),('min-negative',-(1<<63))]:files[label+'.mkv']=clip('fixed',pad)[0]
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9/8kHz mono PCM, three20ms frames in fixed/Xiph/EBML laces. Signed block-level padding crosses lace boundaries; independent sample oracles. No encoder/private media/network.','spec':'https://www.matroska.org/technical/elements.html#DiscardPadding','cases':cases,'invalid':['too-large.mkv','min-negative.mkv'],'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
