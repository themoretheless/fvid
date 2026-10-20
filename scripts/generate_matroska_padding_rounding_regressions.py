"""Authored20ms PCM/Opus video with padding rounded from known sample counts."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_matroska_enabled_track_regressions import video
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-padding-rounding'
def clip(rate,trim,negative,opus=False,padding_override=None):
    count=rate//50;values=[(i*37)%32768-16384 for i in range(count)]
    if trim:
        for i in range(trim):values[i if negative else count-1-i]=0
    padding=(trim*1_000_000_000+rate//2)//rate
    if negative:padding=-padding
    if padding_override is not None:padding=padding_override
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',20.)))
    audio=ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_OPUS' if opus else b'A_PCM/INT/LIT')+ebml('23e383',(20_000_000).to_bytes(4,'big'))
    if opus:audio+=ebml('63a2',b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',0,48000,0,0))
    audio+=ebml('e1',ebml('b5',struct.pack('>d',float(rate)))+ebml('9f',b'\1')+ebml('6264',b'\x10'))
    payload=bytes.fromhex('f8fffe') if opus else b''.join(struct.pack('<h',v) for v in values)
    block=ebml('a0',ebml('a1',b'\x82\0\0\0'+payload)+ebml('9b',b'\x14')+ebml('75a2',padding.to_bytes(8,'big',signed=True)))
    blocks=ebml('e7',b'\0')+block+ebml('a3',b'\x81\0\0\x80'+frame(8,'420',16,16))
    return header+ebml('18538067',info+ebml('1654ae6b',video(1,16,b'\1')+ebml('ae',audio))+ebml('1f43b675',blocks)),b''.join(struct.pack('<f',v/32768.) for v in values),padding

def vorbis_clip(trim,delay_ns=0):
    from generate_vorbis_timestamps import bits
    ident=b'\x01vorbis'+struct.pack('<IBIiiiBB',0,2,48000,0,0,0,0x86,1)
    comment=b'\x03vorbis'+bytes(8)+b'\x01'
    fields=[(5,8)]+[(b,8)for b in b'vorbis']+[(0,8),(0x564342,24),(1,16),(1,24),(0,1),(0,1),(0,5),(0,4),(0,6),(0,16),(0,6),(1,16),(0,5),(0,2),(6,4),(0,6),(0,16),(0,24),(128,24),(31,24),(0,6),(0,8),(0,3),(0,1),(0,6),(0,16),(0,1),(0,1),(0,2),(0,8),(0,8),(0,8),(2,6)]
    for long in [0,0,1]:fields.extend([(long,1),(0,16),(0,16),(0,8)])
    setup=bits(fields+[(1,1)]);private=bytes([2,len(ident),len(comment)])+ident+comment+setup
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',b'\1')+ebml('4489',struct.pack('>d',20_000_000.)))
    audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_VORBIS')+ebml('63a2',private)+(ebml('56aa',delay_ns.to_bytes(8,'big')) if delay_ns else b'')+ebml('e1',ebml('b5',struct.pack('>d',48000.))+ebml('9f',b'\2')))
    clusters=ebml('1f43b675',ebml('e7',b'\0')+ebml('a3',b'\x81\0\0\x80'+frame(8,'420',16,16)))
    modes=[0,2,1,2,2,0];starts=[0,0,80,160,240,368]
    for index,mode in enumerate(modes):
        fields=[(0,1),(mode,2)]
        if mode==2:fields.extend([(int(index>0 and modes[index-1]==2),1),(int(index+1<len(modes) and modes[index+1]==2),1)])
        payload=bits(fields+[(0,1),(0,1)])
        block=b'\x82\0\0\0'+payload
        if index==5:
            padding=(trim*1_000_000_000+24000)//48000
            block=ebml('a0',ebml('a1',block)+ebml('9b',(1_666_667).to_bytes(4,'big'))+ebml('75a2',padding.to_bytes(8,'big',signed=True)))
        else:block=ebml('a3',block[:3]+b'\x80'+block[4:])
        stamp=starts[index]*1_000_000_000//48000
        clusters+=ebml('1f43b675',ebml('e7',stamp.to_bytes(max(1,(stamp.bit_length()+7)//8),'big'))+block)
    return header+ebml('18538067',info+ebml('1654ae6b',video(1,16,b'\1')+audio)+clusters)

def generate():
    files={};cases=[]
    for rate,trim,opus in [(48000,1,False),(48000,2,False),(48000,3,False),(44100,1,False),(44100,2,False),(44100,4,False),(48000,2,True)]:
        for negative in [False,True]:
            name=('opus' if opus else 'pcm')+f'-{rate}-{trim}-'+('head' if negative else 'tail')
            media,pcm,padding=clip(rate,trim,negative,opus);files[name+'.mkv']=media
            if not opus:
                files[name+'-source.f32le']=pcm
                files[name+'-expected.f32le']=pcm[trim*4:] if negative else pcm[:-trim*4]
            cases.append({'file':name+'.mkv','name':name,'rate':rate,'trim':trim,'negative':negative,'opus':opus,'padding_ns':padding})
    files['opus-control.mkv']=clip(48000,0,False,True)[0]
    files['vorbis-control.mkv']=vorbis_clip(0)
    files['vorbis-two-sample-tail.mkv']=vorbis_clip(2)
    for name,padding in [('oversized-tail',20_000_001),('oversized-head',-20_000_001)]:
        files[name+'.mkv']=clip(48000,960,padding<0,padding_override=padding)[0]
    files['manifest.json']=(json.dumps({'provenance':'Authored20ms VP9 plus PCM with exactly known zero padding samples, or synthetic Opus DTX with zero preskip. Signed padding metadata is nearest-ns rounding of the known sample count. No encoder/private media/network.','cases':cases,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
