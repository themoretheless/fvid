"""Owned player dialogue-policy witnesses; offline synthetic movies only."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'dolby-player-normalization'
def generate():
 files={};cases=[]
 for label,folder,name,exts,mask in [
  ('ac3-dual','ac3-dialogue-normalization','l0-1-r1-d27',['mkv','mp4'],0xb),
  ('ac3-loud','ac3-dialogue-normalization','l0-0-r0-d27-loud',['mkv','mp4'],3),
  ('eac3-dual','eac3-dialogue-normalization','l0-1-c127-transient-d27',['mkv'],0xb),
  ('eac3-expanded','eac3-dialogue-expanded','51-lc-44100-6-1-0-switch',['mkv','mp4'],0x63f),
  ('eac3-loud','eac3-dialogue-expanded','51-lc-44100-6-1-1-switch',['mkv','mp4'],0x63f)]:
  for ext in exts:
   filename=f'{label}.{ext}';files[filename]=(ROOT/folder/f'{name}.{ext}').read_bytes();cases.append(dict(file=filename,mask=mask,seek_ms=40))
 files['pcm.mkv']=(ROOT/'dolby-player-compression/pcm.mkv').read_bytes();cases.append(dict(file='pcm.mkv',mask=None,seek_ms=40,dolby=False))
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
