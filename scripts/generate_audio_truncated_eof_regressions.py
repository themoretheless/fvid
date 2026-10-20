"""Author known-truncated and unknown-length WAVE/AU/AIFF completion controls."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
from generate_wav_unsigned8_regressions import wav
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-truncated-eof'
def generate():
    p=pcm();files={'video.mkv':p['video.mkv'],'full.f32le':p['expected.f32le'],'prefix.f32le':p['expected.f32le'][:320]};cases=[]
    for kind,source in [('wav','wav-int16.wav'),('au','au-int16.au')]:
        full=p[source];truncated=full[:-160]
        prefix=wav(8000,1,16,full[44:204]) if kind=='wav' else struct.pack('>6I',0x2e736e64,24,160,3,8000,1)+full[24:184]
        unknown=bytearray(prefix);struct.pack_into('<I' if kind=='wav' else '>I',unknown,40 if kind=='wav' else 8,0xffffffff)
        for label,data,expected,failed in [('full',full,'full.f32le',False),('prefix',prefix,'prefix.f32le',False),('unknown',bytes(unknown),'prefix.f32le',False),('truncated',truncated,'prefix.f32le',True)]:
            name=kind+'-'+label;files[name+'.'+kind]=data;cases.append({'name':name,'kind':kind,'file':name+'.'+kind,'expected':expected,'truncated':failed})
        if kind=='au':
            zero=bytearray(prefix);struct.pack_into('>I',zero,8,0);files['au-unknown-zero.au']=bytes(zero);cases.append({'name':'au-unknown-zero','kind':'au','file':'au-unknown-zero.au','expected':'prefix.f32le','truncated':False})
    full=p['aiff-int16.aiff'];truncated=full[:-160];prefix=bytearray(truncated)
    assert prefix[12:16]==b'COMM' and prefix[38:42]==b'SSND'
    struct.pack_into('>I',prefix,4,len(prefix)-8);struct.pack_into('>I',prefix,22,80);struct.pack_into('>I',prefix,42,168)
    zero=bytearray(prefix);struct.pack_into('>I',zero,22,0)
    for label,data,expected,failed in [('full',full,'full.f32le',False),('prefix',bytes(prefix),'prefix.f32le',False),('zero-claim',bytes(zero),'prefix.f32le',False),('truncated',truncated,'prefix.f32le',True)]:
        name='aiff-'+label;files[name+'.aiff']=data;cases.append({'name':name,'kind':'aiff','file':name+'.aiff','expected':expected,'truncated':failed})
    files['manifest.json']=(json.dumps({'provenance':'Authored20ms8k PCM and matching VP9/PCM video. Known-truncated WAV/AU retain320byte audio claims and AIFF retains160frame COMM claims, but each contains only160bytes/80completeframes. Explicitly shorter and unknown-size sentinel controls remain successful. Prefix decoding is preserved; known truncation must not become successful playback EOF. No encoder/private media/network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
