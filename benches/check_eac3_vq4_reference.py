"""Optional numeric-reference discrepancy experiment, never a production fallback.
Compare FFmpeg AHT PCM against conventional controls with normative vs captured
reference book-4 data. No fixture or production code is changed.
"""
import argparse,json,re,struct,subprocess,sys,tempfile,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'scripts'))
import generate_eac3_aht_quantizers as g
from generate_eac3_bsid import movie
p=argparse.ArgumentParser();p.add_argument('reference_data',type=Path);p.add_argument('--ffmpeg',default='ffmpeg');a=p.parse_args()
source=a.reference_data.read_text();body=re.search(r'vq_hebap4\[.*?=\s*\{(.*?)\n\};',source,re.S).group(1)
nums=[int(v)for v in re.findall(r'-?\d+',body)];rows=[nums[i:i+6]for i in range(0,len(nums),6)]
assert len(rows)==31, 'experiment specifically requires captured 31-row reference'
books=json.loads((ROOT/'scripts/eac3_aht_author_codebooks.json').read_text())['books'];books['4']=rows+[[0]*6]
m=json.loads((g.OUT/'manifest.json').read_text());names=['o-448-g0-t0-headroom','o-384-g0-t0-headroom','o-352-g0-t0-headroom'];results=[]
def decode(path):
 raw=subprocess.run([a.ffmpeg,'-v','error','-i',str(path),'-map','0:a:0','-f','f32le','-'],check=True,stdout=subprocess.PIPE).stdout
 return struct.unpack('<'+'f'*(len(raw)//4),raw)
def error(x,y):
 assert len(x)==len(y)
 return max(abs(a-b)for a,b in zip(x,y))
with tempfile.TemporaryDirectory(prefix='fvid-vq4-reference-') as tmp:
 for c in m['cases']:
  if c['name']not in names:continue
  authored=[g.vectors(c['pointers'],c['mode'],c['seed']+ch,books)for ch in range(2)]
  words=[v[0]for v in authored];values=[v[1]for v in authored]
  shifts=[max(0,math.ceil(math.log2(max(1,max(abs(g.conventional(v,b))for v in values[ch]for b in range(6))/32760))))for ch in range(2)]
  raw=g.packet(False,c['offset'],words,values,shifts,True,c['exponent'])
  path=Path(tmp)/'reference-table-control.mkv';path.write_bytes(movie([raw]*3))
  actual=decode(g.OUT/c['video']);normative=decode(g.OUT/c['control_video']);captured=decode(path)
  results.append(dict(name=c['name'],normative_control_error=error(actual,normative),captured_table_control_error=error(actual,captured)))
print(json.dumps(dict(cases=results),indent=2))
