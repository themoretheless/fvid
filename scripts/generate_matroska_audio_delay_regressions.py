"""Authored general CodecDelay timing and negative presentation windows."""
import hashlib,json
from pathlib import Path
from generate_matroska_pcm_clock_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-audio-delay'
def generate():
    files={};cases=[]
    for name,delay,padding in [('control',0,(0,0,0)),('one-ns',1,(0,0,0)),('ten-ms',10_000_000,(0,0,0)),('whole-head',20_000_000,(0,0,0)),('all-preroll',100_000_000,(0,0,0)),('padding-overlap',10_000_000,(-10_000_000,0,10_000_000))]:
        data,raw=clip(padding=padding,codec_delay_ns=delay);files[name+'.mkv']=data;expected=bytearray();packets=[]
        for index,pad in enumerate(padding):
            source=index*20_000_000-delay
            near=(max(0,-source)*8000+500_000_000)//1_000_000_000
            floor=near if source+near*125_000>=-1 else (max(0,-source)*8000+999_999_999)//1_000_000_000
            trim=(abs(pad)*8000+500_000_000)//1_000_000_000
            first=max(floor,trim if pad<0 else 0);last=160-(trim if pad>0 else 0)
            if first<last:
                expected+=raw[(index*160+first)*4:(index*160+last)*4]
                packets.append({'pts_ns':max(0,source+first*125_000),'frames':last-first})
        files[name+'.f32le']=bytes(expected);cases.append({'file':name+'.mkv','oracle':name+'.f32le','delay_ns':delay,'packets':packets,'end_ns':max([0]+[p['pts_ns']+p['frames']*125_000 for p in packets])})
    files['overflow-delay.mkv']=clip(codec_delay_ns=2**64-1)[0]
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9/8kHz mono PCM. CodecDelay shifts source presentation before clipping. One-ns, whole-packet/all-preroll and padding overlap controls. No encoder/private media/network.','cases':cases,'invalid':'overflow-delay.mkv','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
