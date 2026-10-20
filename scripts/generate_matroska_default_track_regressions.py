"""Authored Matroska default selection; all streams remain usable/manual."""
import json,hashlib
from pathlib import Path
from generate_matroska_enabled_track_regressions import clip,audiovisual
from generate_mp4_matroska_usability_regressions import generate as mp4_sources
from generate_mp4_enabled_track_regressions import boxes,rewrite_track
from generate_audio_resample_window_fixture import atom
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-default-tracks'
def generate():
    files={};cases=[]
    for name,a,b,late,side in [('prefer-second',b'\0',b'\1',False,64),('prefer-first',b'\1',b'\0',False,16),('both-default',b'\1',b'\1',False,16),('no-default',b'\0',b'\0',False,16),('empty-first',b'',b'\1',False,16),('empty-second',b'\0',b'',False,64),('missing-first',None,b'\0',False,16),('late-prefer-second',b'\0',b'\1',True,64)]:
        filename=name+'.mkv';files[filename]=clip(b'\1',b'\1',late=late,defaults=(a,b));cases.append({'file':filename,'side':side,'defaults':[True if a is None or a==b'' else a!=b'\0',True if b is None or b==b'' else b!=b'\0']})
    files['unsupported-default.mkv']=clip(b'\1',b'\1',defaults=(b'\0',b'\1'),unsupported_second=True)
    cases.append({'file':'unsupported-default.mkv','side':16,'defaults':[False,True]})
    files['invalid-value.mkv']=clip(b'\1',b'\1',defaults=(b'\2',b'\1'))
    files['duplicate-default.mkv']=clip(b'\1',b'\1',defaults=(b'\0',b'\1'),duplicate_default=True)
    files['audiovisual.mkv'],files['expected-audio.f32le']=audiovisual(all_usable=True,defaults=(b'\0',b'\1'))
    original=mp4_sources()['disabled-first.mp4'];parts=dict(boxes(original));movie=list(boxes(parts[b'moov']));tracks=[]
    for kind,body in movie:
        if kind!=b'trak':tracks.append(atom(kind,body));continue
        index=sum(1 for k,_ in boxes(b''.join(tracks)) if k==b'trak')
        mdia=dict(boxes(body))[b'mdia'];stbl=dict(boxes(dict(boxes(mdia))[b'minf']))[b'stbl'];offset=int.from_bytes(dict(boxes(stbl))[b'stco'][8:12],'big')
        tracks.append(atom(kind,rewrite_track(body,index+1,5 if index==0 else 3,offset,False)))
    files['movie-priority.mp4']=atom(b'ftyp',parts[b'ftyp'])+atom(b'mdat',parts[b'mdat'])+atom(b'moov',b''.join(tracks))
    files['manifest.json']=(json.dumps({'provenance':'Nine authored60ms VP9/unknown-codec pairs and video/two PCM/subtitle tracks. All usable; default flags alone control priority. Empty/default values and late tracks. No private media/encoder/network.','spec':'https://www.matroska.org/technical/elements.html#FlagDefault','cases':cases,'invalid':[{'file':'invalid-value.mkv','error':'invalid Matroska FlagDefault'},{'file':'duplicate-default.mkv','error':'duplicate Matroska FlagDefault'}],'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
