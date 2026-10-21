"""Optional standalone reference qualification; not fixture generation or ordinary tests.
The supplied executable is built from wavpack_hybrid_reference.c against a pinned
external WavPack checkout. No dependency is added to FVid. Output floats use host
endianness (the current fixture oracle is little endian).
"""
import argparse,hashlib,json,subprocess,sys,tempfile
from pathlib import Path

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--oracle',type=Path,required=True)
 parser.add_argument('--reference-commit',required=True)
 parser.add_argument('--lossy',action='store_true')
 parser.add_argument('--correction',action='store_true')
 parser.add_argument('--fixtures',default='wavpack-hybrid')
 parser.add_argument('--report',type=Path,required=True)
 args=parser.parse_args()
 if sys.byteorder!='little':parser.error('reference harness requires a little-endian host')
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'/args.fixtures
 manifest=json.loads((root/'manifest.json').read_text());results=[]
 with tempfile.TemporaryDirectory(prefix='fvid-wavpack-hybrid-reference-')as work:
  source=Path(work)/'input.wv';output=Path(work)/'output.f32le'
  for case in manifest['cases']:
   source.write_bytes(b''.join((root/f['file']).read_bytes()for f in case['frames']))
   paired=args.correction and not args.lossy and not case.get('main_only',False)
   (Path(work)/'input.wvc').unlink(missing_ok=True)
   if paired:(Path(work)/'input.wvc').write_bytes(b''.join((root/f['correction']).read_bytes()for f in case['frames']))
   expected=(root/case['lossy'if args.lossy else'oracle']).read_bytes();count=len(expected)//4//case.get('channels',2)
   run=subprocess.run([str(args.oracle.resolve()),str(source),str(output),str(count)]+(['correction']if paired else[]),capture_output=True)
   actual=output.read_bytes()if run.returncode==0 else b''
   results.append(dict(name=case['name'],exit=run.returncode,equal=run.returncode==0 and actual==expected,error=run.stderr.decode(errors='replace'),pcm_sha256=hashlib.sha256(actual).hexdigest()))
 args.report.write_text(json.dumps(dict(reference_commit=args.reference_commit,oracle_sha256=hashlib.sha256(args.oracle.read_bytes()).hexdigest(),cases=results),indent=2)+'\n')
 accepted=sum(c['equal']for c in results);print(f'{accepted}/{len(results)} exact, no CRC errors')
 for c in results:
  if not c['equal']:print(c['name'],c['error'])
 return int(accepted!=len(results))
if __name__=='__main__':raise SystemExit(main())
