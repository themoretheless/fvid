"""Authored Opus laces with known audible endpoints and discarded packet tails."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_matroska_enabled_track_regressions import video
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-opus-endpoint'
def clip(mode,padding,delay=0,origin=0,preskip=0,default_duration=0):
    variable=mode in ['xiph','ebml']
    packets=[bytes.fromhex(v) for v in (['f8fffe','f9fffefffe','f0fffe','f8fffe'] if variable else ['f8fffe']*3)]
    durations=[20,40,10,20] if variable else [20,20,20]
    total=sum(durations);frames=(origin+total+19)//20
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',float(frames*20))))
    head=b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',preskip,48000,0,0)
    audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_OPUS')+ebml('63a2',head)+ebml('56aa',delay.to_bytes(8,'big'))+(ebml('23e383',default_duration.to_bytes(8,'big')) if default_duration else b'')+ebml('e1',ebml('b5',struct.pack('>d',48000.))+ebml('9f',b'\1')))
    if mode=='plain':
        trims=[0]*len(packets);remaining=padding
        for i in range(len(packets)-1,-1,-1):trims[i]=min(remaining,durations[i]*1_000_000);remaining-=trims[i]
        blocks=ebml('e7',b'\0');clock=origin
        for packet,duration,trim in zip(packets,durations,trims):
            block=b'\x82'+struct.pack('>h',clock)+b'\0'+packet
            blocks+=ebml('a0',ebml('a1',block)+ebml('9b',bytes([duration]))+ebml('75a2',trim.to_bytes(8,'big',signed=True))) if trim else ebml('a3',block[:3]+b'\x80'+block[4:])
            clock+=duration
    else:
        flags,lace={'fixed':(4,bytes([2])),'xiph':(2,bytes([3,3,5,3])),'ebml':(6,bytes([3,0x83,0xc1,0xbd]))}[mode]
        block=b'\x82'+struct.pack('>h',origin)+bytes([flags])+lace+b''.join(packets)
        blocks=ebml('e7',b'\0')+ebml('a0',ebml('a1',block)+ebml('9b',bytes([total]))+ebml('75a2',padding.to_bytes(8,'big',signed=True)))
    for index in range(frames):blocks+=ebml('a3',b'\x81'+struct.pack('>h',index*20)+b'\x80'+frame(8,'420',16,16))
    return header+ebml('18538067',info+ebml('1654ae6b',video(1,16,b'\1')+audio)+ebml('1f43b675',blocks)),durations,frames

def generate():
    files={};cases=[]
    for mode in ['fixed','xiph','ebml','plain']:
        total=90 if mode in ['xiph','ebml'] else 60
        choices=[('control',0,0,0,0),('one-tail',20_000_000,0,0,0),('several-tail',(35 if mode in ['xiph','ebml'] else 40)*1_000_000,0,0,0),('whole',total*1_000_000,0,0,0)]
        if mode=='fixed':choices += [('delay-two',0,41667,0,0),('positive-origin',0,0,100,0),('header-preskip',0,0,0,312),('preskip-tail',40_000_000,0,0,312)]
        for label,padding,delay,origin,preskip in choices:
            name=mode+'-'+label;data,durations,frames=clip(mode,padding,delay,origin,preskip);files[name+'.mkv']=data
            effective_delay=delay if delay else preskip*1_000_000_000//48000
            first=max(preskip,(effective_delay*48000+500_000_000)//1_000_000_000) if origin==0 else preskip
            last=sum(durations)*48-padding*48000//1_000_000_000
            end=max(0,(origin+sum(durations))*1_000_000-effective_delay-padding) if first<last else 0
            cases.append({'name':name,'file':name+'.mkv','control':mode+'-control.mkv','mode':mode,'video_frames':frames,'total_frames':sum(durations)*48,'first':first,'last':last,'end_ns':end,'indexed_known':mode!='plain'})
    files['manifest.json']=(json.dumps({'provenance':'Authored60/90ms Opus DTX fixed/Xiph/EBML laces with VP9 video. Known TOC durations, fully discarded tail packets, whole padding, codec delay and positive origin. No encoder/private media/network.','cases':cases,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
