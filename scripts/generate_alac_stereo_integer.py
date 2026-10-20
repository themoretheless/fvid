"""Full32 stereo matrix/low-bit integer fixtures, authored offline."""
import json,struct
from pathlib import Path
import generate_alac_multichannel_fixtures as g
from generate_alac_cookie_wrappers import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-stereo-integer'
def pcm(n):
    sign=-1 if n%2 else 1
    return [((sign*(0x400001+n*7))<<8)|(n*13%256),
            ((-sign*(0x400013+n*3))<<8)|((255-n*17)%256)]
def generate():
    files={};cases=[]
    cookie=bytearray(24);cookie[:4]=g.word(16)
    cookie[5:10]=bytes([32,40,10,14,2]);cookie[20:24]=g.word(48000)
    expected=struct.pack('<64i',*(v for n in range(32)for v in pcm(n)))
    for extra in [8,16]:
        for weight in [-128,-127,-64,-1,0,1,64,127]:
            for shift in [7,31]:
                packets=[];name=(''if extra==8 else'low16-')+f'weight{weight}-shift{shift}'
                for start in [0,16]:
                    bits='001'+'0000'+'0'*12+'1'+f'{extra//8:02b}'+'0'+f'{16:032b}'
                    bits+=f'{shift:08b}{weight&255:08b}'+('0000'+'0001'+'100'+'00000')*2
                    planes=[[],[]]
                    for n in range(start,start+16):
                        left,right=pcm(n);high=[left>>extra,right>>extra];delta=high[0]-high[1]
                        product=((delta*weight+0x80000000)&0xffffffff)-0x80000000
                        mixed=[high[1]+(product>>shift),delta]if weight else high
                        for ch,v in enumerate(mixed):planes[ch].append(v)
                        bits+=f'{left&((1<<extra)-1):0{extra}b}{right&((1<<extra)-1):0{extra}b}'
                    stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
                    for plane in planes:bits+=g.rice_plane(plane,33-extra,stats)
                    bits+='111';bits+='0'*(-len(bits)%8)
                    packets.append(int(bits,2).to_bytes(len(bits)//8,'big'))
                files[name+'.mkv']=movie(cookie,2,packets)
                files[name+'.m4a']=g.mp4(cookie,2,32,packets)
                for i,p in enumerate(packets):files[name+f'.packet{i}']=p
                cases.append(dict(name=name,weight=weight,shift=shift,extra=extra))
    files['expected.i32le']=expected;files['cookie']=bytes(cookie)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Authored stereo PCM, varying low bytes, signed matrix weights, neutral VP9. Offline own generator only.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
