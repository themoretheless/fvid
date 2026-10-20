"""Author AU u32 channel counts that exceed the AudioStream u16 interface."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
from generate_pcm_run_alignment_regressions import generate as aligned
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/au-channel-range'
def au(channels):
    payload=bytes(channels) # One complete, silent, signed8 frame in the declared layout.
    return struct.pack('>6I',0x2e736e64,24,len(payload),2,8000,channels)+payload

def generate():
    p=pcm();a=aligned();files={'video.mkv':p['video.mkv'],'mono.au':p['au-int8.au'],'mono.f32le':p['expected.f32le'],'stereo.au':a['au-int8-stereo.au'],'stereo.f32le':a['au-int8-stereo.f32le'],'representable-max.au':au(65535)}
    bad=[]
    for channels in [65536,65537,65538,131073]:name=f'channels-{channels}.au';files[name]=au(channels);bad.append({'file':name,'channels':channels})
    files['manifest.json']=(json.dumps({'provenance':'Authored AU signed8 with one complete silent frame at u32 channel counts, bounded files131097bytes maximum. Above-u16 geometry must refuse even with a permissive channel budget. Mono/stereo numeric native controls and matching short VP9/PCM video. The65535boundary control is metadata/packet geometry, not physical-device acceptance. No private media, encoder or network.','bad':bad,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
