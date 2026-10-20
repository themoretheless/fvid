"""Authored two-video Matroska automatic-selection witnesses."""
import json,hashlib,struct
from pathlib import Path
from generate_vp9_chroma import frame
from generate_audio_resample_window_fixture import ebml
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-enabled-tracks'
def video(number,side,enabled,duplicate=False,default_flag=None,duplicate_default=False,codec=b'V_VP9'):
    data=ebml('d7',bytes([number]))+ebml('73c5',bytes([number]))+ebml('83',b'\1')+ebml('86',codec)+ebml('23e383',(20_000_000).to_bytes(4,'big'))+ebml('e0',ebml('b0',bytes([side]))+ebml('ba',bytes([side])))
    if enabled is not None:data+=ebml('b9',enabled)
    if duplicate:data+=ebml('b9',b'\1')
    if default_flag is not None:data+=ebml('88',default_flag)
    if duplicate_default:data+=ebml('88',b'\1')
    return ebml('ae',data)
def clip(first,second=b'\1',late=False,corrupt=False,duplicate=False,defaults=(None,None),duplicate_default=False,unsupported_second=False):
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',60.))+ebml('4d80',b'fvid synthetic')+ebml('5741',b'fvid synthetic'))
    tracks=ebml('1654ae6b',video(1,16,first,duplicate,defaults[0],duplicate_default)+video(2,64,second,default_flag=defaults[1],codec=b'V_FVID_UNKNOWN' if unsupported_second else b'V_VP9'))
    blocks=ebml('e7',b'\0')
    for index in range(3):
        for track,side in [(1,16),(2,64)]:blocks+=ebml('a3',bytes([0x80|track])+struct.pack('>h',index*20)+b'\x80'+(b'\xff' if corrupt and track==1 else frame(8,'420',side,side)))
    cluster=ebml('1f43b675',blocks)
    return header+ebml('18538067',info+(cluster+tracks if late else tracks+cluster))
def audiovisual(all_usable=False,defaults=(None,None),audio_widths=(16,16),audio_codecs=(b'A_PCM/INT/LIT',b'A_PCM/INT/LIT')):
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',60.))+ebml('4d80',b'fvid synthetic')+ebml('5741',b'fvid synthetic'))
    tracks=video(1,16,b'\1')
    for number,kind,codec in [(2,2,b'A_PCM/INT/LIT'),(3,2,b'A_PCM/INT/LIT'),(4,17,b'S_TEXT/UTF8'),(5,17,b'S_TEXT/UTF8')]:
        if kind==2:codec=audio_codecs[number-2]
        track=ebml('d7',bytes([number]))+ebml('73c5',bytes([number]))+ebml('83',bytes([kind]))+ebml('86',codec)+ebml('b9',bytes([1 if all_usable else number%2]))+ebml('536e',b'enabled' if number%2 else b'disabled')+ebml('23e383',(20_000_000).to_bytes(4,'big'))
        if defaults[(number-2)%2] is not None:track+=ebml('88',defaults[(number-2)%2])
        if kind==2:track+=ebml('e1',ebml('b5',struct.pack('>d',8000.))+ebml('9f',b'\1')+ebml('6264',audio_widths[number-2].to_bytes(max(1,(audio_widths[number-2].bit_length()+7)//8),'big')))
        tracks+=ebml('ae',track)
    blocks=ebml('e7',b'\0');expected=bytearray()
    for index in range(3):
        samples=[((index*160+i)*37)%32768-16384 for i in range(160)]
        for value in samples:expected+=struct.pack('<f',value/32768.)
        for number,payload in [(1,frame(8,'420',16,16)),(2,bytes(320)),(3,b''.join(struct.pack('<h',v) for v in samples)),(4,b'disabled text'),(5,f'enabled text{index}'.encode())]:
            group=ebml('a1',bytes([0x80|number])+struct.pack('>h',index*20)+b'\0'+payload)+ebml('9b',b'\x14');blocks+=ebml('a0',group)
    return header+ebml('18538067',info+ebml('1654ae6b',tracks)+ebml('1f43b675',blocks)),bytes(expected)
def generate():
    files={};cases=[]
    for name,first,second,late,corrupt,side in [
        ('omitted',None,b'\1',False,False,16),('empty',b'',b'\1',False,False,16),('enabled',b'\1',b'\1',False,False,16),
        ('disabled',b'\0',b'\1',False,False,64),('late-disabled',b'\0',b'\1',True,False,64),
        ('disabled-corrupt',b'\0',b'\1',False,True,64),('all-disabled',b'\0',b'\0',False,False,None)]:
        filename=name+'.mkv';files[filename]=clip(first,second,late,corrupt);cases.append({'file':filename,'side':side})
    files['audiovisual.mkv'],files['expected-audio.f32le']=audiovisual()
    files['invalid-value.mkv']=clip(b'\2')
    files['duplicate-flag.mkv']=clip(b'\0',duplicate=True)
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms neutral16/64-square VP9 tracks with explicit/missing/empty usability flags. Disabled-corrupt case tests that disabled packet data is not decoded. No private media/encoder/network.','spec':'https://www.matroska.org/technical/elements.html#FlagEnabled','cases':cases,'invalid':[{'file':'invalid-value.mkv','error':'invalid Matroska FlagEnabled'},{'file':'duplicate-flag.mkv','error':'duplicate Matroska FlagEnabled'}],'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
