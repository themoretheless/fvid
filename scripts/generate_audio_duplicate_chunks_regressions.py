"""Author short RIFF/AIFF duplicate layout/payload ambiguity reproducers."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-duplicate-chunks'
def chunk(tag,data,big=False):return tag+struct.pack('>I' if big else '<I',len(data))+data+bytes(len(data)%2)
def chunks(data,big=False):
    at=12
    while at<len(data):
        tag=data[at:at+4];size=struct.unpack_from('>I' if big else '<I',data,at+4)[0];yield tag,data[at+8:at+8+size];at+=8+size+(size&1)
    assert at==len(data)
def wave(parts):
    body=b'WAVE'+b''.join(chunk(t,p) for t,p in parts);return b'RIFF'+struct.pack('<I',len(body))+body
def aiff(parts):
    body=b'AIFF'+b''.join(chunk(t,p,True) for t,p in parts);return b'FORM'+struct.pack('>I',len(body))+body

def generate():
    p=pcm();w=dict(chunks(p['wav-int16.wav']));a=dict(chunks(p['aiff-int16.aiff'],True));fmt=w[b'fmt '];data=w[b'data'];comm=a[b'COMM'];sound=a[b'SSND'];files={'video.mkv':p['video.mkv'],'expected.f32le':p['expected.f32le']};cases=[]
    junk=(b'JUNK',b'fmt data COMM SSND!')
    different_fmt=bytearray(fmt);struct.pack_into('<II',different_fmt,4,16000,32000)
    different_comm=bytearray(comm);different_comm[8:18]=bytes.fromhex('400cfa00000000000000')
    def add(name,kind,parts,needle=None):
        files[name+'.'+kind]=(wave if kind=='wav' else aiff)(parts);cases.append({'name':name,'kind':kind,'file':name+'.'+kind,'refusal':needle})
    add('wave-control','wav',[(b'fmt ',fmt),(b'data',data)])
    add('wave-identical-fmt','wav',[(b'fmt ',fmt),junk,(b'fmt ',fmt),(b'data',data)])
    add('wave-identical-fact','wav',[(b'fmt ',fmt),(b'fact',struct.pack('<I',160)),(b'data',data),(b'fact',struct.pack('<I',160))])
    add('wave-empty-fact','wav',[(b'fmt ',fmt),(b'fact',b''),(b'fact',struct.pack('<I',160)),(b'data',data)])
    add('wave-data-first','wav',[(b'data',data),junk,(b'fmt ',fmt)])
    add('wave-split-data','wav',[(b'fmt ',fmt),(b'data',data[:160]),junk,(b'data',data[160:])],'multiple Wave data chunks')
    add('wave-info-list','wav',[(b'fmt ',fmt),(b'LIST',b'INFO'+chunk(b'INAM',b'synthetic wavl metadata\0')),(b'data',data)])
    add('wave-wavl-only','wav',[(b'fmt ',fmt),(b'LIST',b'wavl'+chunk(b'data',data))],'Wave LIST/wavl audio is not supported')
    add('wave-data-plus-wavl','wav',[(b'fmt ',fmt),(b'data',data[:160]),(b'LIST',b'wavl'+chunk(b'data',data[160:]))],'Wave LIST/wavl audio is not supported')
    add('wave-repeated-data','wav',[(b'fmt ',fmt),(b'data',data[:160]),(b'data',data[:160])],'multiple Wave data chunks')
    add('wave-conflicting-fmt','wav',[(b'fmt ',fmt),(b'data',data),(b'fmt ',bytes(different_fmt))],'conflicting Wave fmt chunks')
    add('wave-conflicting-fact','wav',[(b'fmt ',fmt),(b'fact',struct.pack('<I',160)),(b'data',data),(b'fact',struct.pack('<I',80))],'conflicting Wave fact sample counts')
    add('aiff-control','aiff',[(b'COMM',comm),(b'SSND',sound)])
    add('aiff-identical-comm','aiff',[(b'COMM',comm),junk,(b'COMM',comm),(b'SSND',sound)])
    add('aiff-sound-first','aiff',[(b'SSND',sound),junk,(b'COMM',comm)])
    add('aiff-split-sound','aiff',[(b'COMM',comm),(b'SSND',bytes(8)+sound[8:168]),junk,(b'SSND',bytes(8)+sound[168:])],'multiple AIFF SSND chunks')
    add('aiff-repeated-sound','aiff',[(b'COMM',comm),(b'SSND',bytes(8)+sound[8:168]),(b'SSND',bytes(8)+sound[8:168])],'multiple AIFF SSND chunks')
    add('aiff-conflicting-comm','aiff',[(b'COMM',comm),(b'SSND',sound),(b'COMM',bytes(different_comm))],'conflicting AIFF COMM chunks')
    files['manifest.json']=(json.dumps({'provenance':'Authored20ms8k PCM RIFF/AIFF and matching native VP9/PCM. Split/repeated payloads and contradictory layout/count headers must refuse rather than choose a prefix. Identical metadata, empty unstated fact, unknown chunks and reordered header/data remain playback controls. No multipart concatenation acceptance is asserted. No private media, encoder or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
