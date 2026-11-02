"""Authored constant ADPCM/Dolby timelines with native-grid silence and repeats."""
import json,hashlib
from pathlib import Path
from generate_mp4_adpcm_transitions import generate as adpcm
from generate_mp4_ac3_sample_clock import generate as ac3
from generate_mp4_audio_reconfiguration import generate as reconfiguration
from generate_mp4_variable_rate_edits import edit_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-constant-edit-subtick-seek'
def generate():
 a=adpcm();d=ac3();e=reconfiguration();cases=[('ima4',a['control-0.mp4'],2000,8000,1,6),('ima-wav',a['control-1.mp4'],2000,16000,2,6),('ms-adpcm',a['control-2.mp4'],2000,8000,1,6),('ac3',d['exact.mp4'],48000,48000,6,5),('ac3-grouped',d['grouped-exact.mp4'],48000,48000,6,5),('ac3-one-packet',d['one-packet-exact.mp4'],48000,48000,6,5),('eac3-48000',e['eac3-control-1.mp4'],1000000,48000,2,17),('eac3-32000',e['eac3-control-2.mp4'],1000000,32000,1,17)]
 files={n+'.mp4':edit_movie(raw,[(3,-1),(8,scale*4//1000),(2,-1),(8,scale*4//1000)])for n,raw,scale,rate,ch,video in cases}
 files['manifest.json']=(json.dumps(dict(provenance='Repository-authored analytic ADPCM,existing synthetic AC3 5.1 tone and authored EAC3 spectra with neutral VP9. Three-ms silence,eight-ms source from four ms,two-ms silence,repeated source. All edit origins are exactly native aligned. No private media/parameters,FFmpeg,process or network.',cases=[dict(file=n+'.mp4',rate=rate,channels=ch,video_frames=video,native_frames=rate*21//1000)for n,raw,scale,rate,ch,video in cases],sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
