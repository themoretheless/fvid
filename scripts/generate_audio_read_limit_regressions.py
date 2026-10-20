"""Author short video/audio controls for maximum whole-file byte-budget probes."""
import hashlib,json
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
from generate_avi_audio_eof_regressions import generate as avi
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/audio-read-limit'
def generate():
    p=pcm();a=avi();files={'video.mkv':p['video.mkv'],'pcm.f32le':p['expected.f32le'],'ima.f32le':a['ima.f32le']}
    cases=[]
    for kind,source in [('wav','wav-int16.wav'),('aiff','aiff-int16.aiff'),('au','au-int16.au')]:files['control.'+kind]=p[source];cases.append({'kind':kind,'file':'control.'+kind,'expected':'pcm.f32le'})
    files['control.avi']=a['ima.avi'];cases.append({'kind':'avi','file':'control.avi','expected':'ima.f32le'})
    # Existing repository-generated synthetic controls are read in place, never copied.
    for kind,source in [('aac','audio/aac-stereo.aac'),('ac3','audio/ac3-stereo.ac3'),('mp3','mp3/tone.mp3'),('flac','flac/tone.flac'),('smf','midi/two-notes.mid'),('xm','xm/two-notes.xm')]:
        cases.append({'kind':kind,'existing_fixture':source,'sha256':hashlib.sha256((ROOT/'tests/fixtures'/source).read_bytes()).hexdigest()})
    files['manifest.json']=(json.dumps({'provenance':'Authored20ms VP9/PCM companion and WAV/AIFF/AU PCM, authored tiny VP9/AVI IMA blocks. Additional existing synthetic tone/note controls are referenced in place and not copied. Tests compare bounded/exact/maximum reads, packet bytes and clocks; no decoder capability is added. No private media, encoder or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
