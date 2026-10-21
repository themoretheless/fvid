"""Owned MP4 expanded AHT, silent six-block base 5.1 and active rear pair."""
import json
from pathlib import Path
from generate_eac3_presentation import generate as presentation
from generate_eac3_mp4_presentation import rewrite
from generate_eac3_aht_lfe import audiovisual
from generate_eac3_playback import BitWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-mp4-aht'
def generate(*, sounding_base=False, coupled=False):
 files=presentation(expanded_aht_base51="pulsed" if sounding_base or coupled else True,coupled=coupled)
 for c in json.loads(files['manifest.json'])['cases']:
  bundle=b''.join(files[p['raw']]for p in c['components'])
  bitrate=(len(bundle)*8*48000+1535999)//1536000
  w=BitWriter().put(bitrate,13).put(0,3).put(0,2).put(16,5).zeros(5).put(7,3).put(1,1).zeros(3).put(1,4).put(2,9)
  config=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,48,8))
  files[c['name']+'.mp4']=rewrite(audiovisual([bundle]*3),config,48000)
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
