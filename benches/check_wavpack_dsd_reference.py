"""Independent DSD and PCM reference verification, separate from ordinary tests.
Requires the supplied oracle compiled against an explicitly pinned WavPack library.
"""
import argparse,json,subprocess,tempfile,struct,hashlib,sys
from pathlib import Path

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--oracle',type=Path,required=True);p.add_argument('--report',type=Path,required=True);p.add_argument('--reference-commit',required=True);a=p.parse_args()
 if sys.byteorder!='little':p.error('reference PCM requires a little-endian host')
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-dsd';m=json.loads((root/'manifest.json').read_text());results=[]
 with tempfile.TemporaryDirectory(prefix='fvid-dsd-reference-')as work:
  source=Path(work)/'input.wv';out=Path(work)/'output.bin'
  for c in m['cases']:
   source.write_bytes(b''.join((root/f['file']).read_bytes()for f in c['frames']));n=len((root/c['oracle']).read_bytes())//c['channels']
   r=dict(name=c['name'],chunk_frames=c['samples'])
   for mode,key in [('native','oracle'),('pcm','pcm')]:
    call=subprocess.run([str(a.oracle.resolve()),str(source),str(out),str(n),mode,str(c["samples"])],capture_output=True)
    expected=(root/c[key]).read_bytes();actual=out.read_bytes()if call.returncode==0 else b''
    result=dict(exit=call.returncode,equal=call.returncode==0 and actual==expected,error=call.stderr.decode(errors='replace'),sha256=hashlib.sha256(actual).hexdigest())
    if mode=='pcm'and len(actual)==len(expected):result['max_error_lsb']=max(abs(x-y)*(1<<23)for(x,),(y,)in zip(struct.iter_unpack('<f',actual),struct.iter_unpack('<f',expected)))
    r[mode]=result
   results.append(r)
 a.report.write_text(json.dumps(dict(reference_commit=a.reference_commit,oracle_sha256=hashlib.sha256(a.oracle.read_bytes()).hexdigest(),cases=results),indent=2)+'\n')
 for mode in ['native','pcm']:
  print(mode,sum(r[mode]['equal']for r in results),'/',len(results))
  for r in results:
   if not r[mode]['equal']:print(r['name'],mode,r[mode].get('max_error_lsb'),r[mode]['error'])
 return int(any(not r['native']['equal']or not r['pcm']['equal']for r in results))
if __name__=='__main__':raise SystemExit(main())
