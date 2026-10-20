"""Author short variable-block Vorbis laces and VP9 from synthetic bitfields."""
import hashlib,json
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_matroska_padding_rounding_regressions import vorbis_clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/vorbis-lace-clock'
def elements(data):
    at=0
    while at<len(data):
        start=at;first=data[at];width=1
        while not first&(128>>(width-1)):width+=1
        tag=data[at:at+width].hex();at+=width;first=data[at];width=1
        while not first&(128>>(width-1)):width+=1
        size=int.from_bytes(data[at:at+width],'big')&((1<<(7*width))-1);at+=width
        yield tag,data[at:at+size],data[start:at+size];at+=size
    assert at==len(data)
def generate():
    source=vorbis_clip(0);top=list(elements(source));header=top[0][2];parts=list(elements(top[1][1]));prefix=b'';packets=[]
    for tag,payload,raw in parts:
        if tag!='1f43b675':prefix+=raw;continue
        fields=list(elements(payload))
        if any(t=='a3' and p.startswith(b'\x81') for t,p,r in fields):prefix+=raw
        for t,p,r in fields:
            if t=='a0':p=next(p for t,p,r in elements(p) if t=='a1');t='a3'
            if t=='a3' and p.startswith(b'\x82'):packets.append(p[4:])
    assert len(packets)==6 and all(len(p)==1 for p in packets)
    files={'control.mkv':source}
    names=[]
    def group(mode,packets,duration,trim=0):
        count=len(packets)
        flags,lace={'fixed':(4,bytes([count-1])),'xiph':(2,bytes([count-1]+[1]*(count-1))),'ebml':(6,bytes([count-1,0x81]+[0xbf]*(count-2)))}[mode]
        block=b'\x82\0\0'+bytes([flags])+lace+b''.join(packets)
        return ebml('a0',ebml('a1',block)+(ebml('9b',duration.to_bytes(4,'big')) if duration is not None else b'')+(ebml('75a2',((trim*1_000_000_000+24000)//48000).to_bytes(8,'big',signed=True)) if trim else b''))
    for mode in ['fixed','xiph','ebml']:
        for split in [False,True]:
            for declared in [True,False]:
                name=mode+('-split' if split else '')+('' if declared else '-untimed')
                if split:
                    clusters=ebml('1f43b675',ebml('e7',b'\0')+group(mode,packets[:3],3_333_333 if declared else None))
                    clusters+=ebml('1f43b675',ebml('e7',(3_333_333).to_bytes(4,'big'))+group(mode,packets[3:],6_000_000 if declared else None))
                else:clusters=ebml('1f43b675',ebml('e7',b'\0')+group(mode,packets,9_333_333 if declared else None))
                files[name+'.mkv']=header+ebml('18538067',prefix+clusters);names.append(name)
                trimmed=name+'-tail-two'
                if split:
                    tail_clusters=ebml('1f43b675',ebml('e7',b'\0')+group(mode,packets[:3],3_333_333 if declared else None))
                    tail_clusters+=ebml('1f43b675',ebml('e7',(3_333_333).to_bytes(4,'big'))+group(mode,packets[3:],6_000_000 if declared else None,2))
                else:tail_clusters=ebml('1f43b675',ebml('e7',b'\0')+group(mode,packets,9_333_333 if declared else None,2))
                files[trimmed+'.mkv']=header+ebml('18538067',prefix+tail_clusters);names.append(trimmed)
    invalid=list(packets);invalid[1]=b'\x06'
    files['invalid-mode.mkv']=header+ebml('18538067',prefix+ebml('1f43b675',ebml('e7',b'\0')+group('fixed',invalid,9_333_333)))
    files['manifest.json']=(json.dumps({'provenance':'Authored Vorbis modes short/long/short/long/long/short:0/80/80/80/128/80 overlap frames,448frames at48k stereo, plus native VP9. No encoder/private media/network.','cases':names,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
