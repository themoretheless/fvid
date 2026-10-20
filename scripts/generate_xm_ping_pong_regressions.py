"""Author ping-pong loops versus independently unfolded forward loops."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-ping-pong'
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];inst=at+9+struct.unpack_from('<H',base,at+7)[0];header=base[:at];instrument=base[inst:inst+243]
    def module(frames,start,ping,note):
        payload=bytearray();last=0
        for value in frames:
            delta=(value-last+32768)%65536-32768;payload+=struct.pack('<h',delta);last=value
        sample=bytearray(40);struct.pack_into('<III',sample,0,len(payload),start*2,(len(frames)-start)*2);sample[12]=64;sample[14]=16+(2 if ping else 1);sample[15]=128
        rows=[bytes([note,1,0,0,0])]+[bytes(5)]*3+[bytes([97,0,0,0,0])];data=b''.join(rows)
        return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+instrument+bytes(sample)+payload
    files={'video.mkv':authored['video.mkv']};cases=[]
    for start,length,note in [(0,64,49),(8,32,49),(0,3,49),(0,64,96),(0,1,49)]:
        frames=[(i-32)*512 for i in range(start+length)];unfolded=frames+frames[start+1:-1][::-1];name=f'loop-{start}-{length}-{note}'
        files[name+'.xm']=module(frames,start,True,note);files[name+'-forward.xm']=module(unfolded,start,False,note);cases.append(dict(name=name,file=name+'.xm',control=name+'-forward.xm'))
    files['manifest.json']=(json.dumps(dict(provenance='Five authored signed16loops compare ping-pong with independently unfolded forward data,excluding repeated turn endpoints. Loop prefixes,length1/3/32/64andhigh pitch;five rows include release. Independent PCM tolerance1e-6allows floating fold arithmetic. Native VP9/PCM companion,no private data/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
