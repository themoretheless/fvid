"""Authored signed discard padding on short PCM video clips."""
import hashlib,json
from pathlib import Path
from generate_matroska_pcm_clock_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-audio-padding'
def generate():
    files={};cases=[]
    for name,offset,padding in [('control',0,(0,0,0)),('start',0,(-10_000_000,0,0)),('end',0,(0,0,10_000_000)),('both',0,(-10_000_000,0,10_000_000)),('rounding',0,(-1,0,1)),('whole-tail',0,(0,0,20_000_000)),('overlap',-10,(-10_000_000,0,10_000_000))]:
        video,raw=clip(offset=offset,padding=padding);files[name+'.mkv']=video;expected=bytearray();packets=[]
        for index,pad in enumerate(padding):
            source_ms=index*20+offset
            floor=max(0,-source_ms)*8
            trim=(abs(pad)*8000+500_000_000)//1_000_000_000
            first=max(floor,trim if pad<0 else 0);last=160-(trim if pad>0 else 0)
            if first<last:
                expected+=raw[(index*160+first)*4:(index*160+last)*4]
                packets.append({'pts_ns':source_ms*1_000_000+first*125_000,'frames':last-first})
        files[name+'.f32le']=bytes(expected);cases.append({'file':name+'.mkv','oracle':name+'.f32le','packets':packets,'end_ns':max(p['pts_ns']+p['frames']*125_000 for p in packets)})
    files['invalid-padding.mkv']=clip(padding=(20_125_000,0,0))[0]
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9/8kHz mono PCM; signed DiscardPadding trims packet heads/tails. Independent integer sample-window oracles. No encoder/private media/network.','cases':cases,'invalid':'invalid-padding.mkv','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
