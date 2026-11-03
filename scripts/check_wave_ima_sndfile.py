"""Optional libsndfile diagnostics, separate from ordinary offline Rust tests."""
import ctypes as C,ctypes.util,json,struct,argparse
from pathlib import Path
from generate_wave_ima_playback import OUT
class Info(C.Structure):
 _fields_=[('frames',C.c_int64),('samplerate',C.c_int),('channels',C.c_int),('format',C.c_int),('sections',C.c_int),('seekable',C.c_int)]
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--output',required=True);a=p.parse_args();name=ctypes.util.find_library('sndfile');assert name;lib=C.CDLL(name)
 lib.sf_open.argtypes=[C.c_char_p,C.c_int,C.POINTER(Info)];lib.sf_open.restype=C.c_void_p;lib.sf_close.argtypes=[C.c_void_p];lib.sf_readf_float.argtypes=[C.c_void_p,C.POINTER(C.c_float),C.c_int64];lib.sf_readf_float.restype=C.c_int64;lib.sf_strerror.argtypes=[C.c_void_p];lib.sf_strerror.restype=C.c_char_p;lib.sf_version_string.restype=C.c_char_p
 rows=[]
 for c in json.loads((OUT/'manifest.json').read_text())['cases']:
  if c['kind']!='riff':continue
  info=Info();handle=lib.sf_open(str(OUT/c['file']).encode(),0x10,C.byref(info))
  if not handle:rows.append(dict(file=c['file'],error=lib.sf_strerror(None).decode()));continue
  try:
   assert 0<info.frames<1000000 and 0<info.channels<=2
   buf=(C.c_float*(info.frames*info.channels))();n=lib.sf_readf_float(handle,buf,info.frames);data=b''.join(struct.pack('<f',v)for v in buf[:n*info.channels]);gold=(OUT/c['oracle']).read_bytes()
   rows.append(dict(file=c['file'],mode=c['mode'],reported_frames=info.frames,read_frames=n,expected_frames=c['frames'],byte_identical=data==gold,prefix_identical=data[:len(gold)]==gold))
  finally:lib.sf_close(handle)
 report=dict(library=name,version=lib.sf_version_string().decode(),cases=rows);Path(a.output).write_text(json.dumps(report,indent=2)+'\n');print('libsndfile exact',sum(r.get('byte_identical',False)for r in rows),'/',len(rows))
