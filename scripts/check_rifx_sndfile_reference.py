"""Optional libsndfile reference;ordinary Rust tests do not load this library."""
import argparse,ctypes,hashlib,json,struct
from pathlib import Path
from generate_rifx_playback import OUT
class Info(ctypes.Structure):
 _fields_=[('frames',ctypes.c_int64),('samplerate',ctypes.c_int),('channels',ctypes.c_int),('format',ctypes.c_int),('sections',ctypes.c_int),('seekable',ctypes.c_int)]
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--library',required=True);p.add_argument('--output',required=True);a=p.parse_args();lib=ctypes.CDLL(a.library);lib.sf_version_string.restype=ctypes.c_char_p;lib.sf_open.argtypes=[ctypes.c_char_p,ctypes.c_int,ctypes.POINTER(Info)];lib.sf_open.restype=ctypes.c_void_p;lib.sf_readf_float.argtypes=[ctypes.c_void_p,ctypes.POINTER(ctypes.c_float),ctypes.c_int64];lib.sf_readf_float.restype=ctypes.c_int64;lib.sf_close.argtypes=[ctypes.c_void_p];lib.sf_strerror.argtypes=[ctypes.c_void_p];lib.sf_strerror.restype=ctypes.c_char_p;results=[]
 for c in json.loads((OUT/'manifest.json').read_text())['cases']:
  f=OUT/(c['name']+'.rifx.wav');expected=(OUT/(c['name']+'.f32le')).read_bytes();info=Info();handle=lib.sf_open(str(f).encode(),0x10,ctypes.byref(info));entry=dict(file=f.name,opened=bool(handle),source_sha256=hashlib.sha256(f.read_bytes()).hexdigest(),expected_sha256=hashlib.sha256(expected).hexdigest(),frames=info.frames,rate=info.samplerate,channels=info.channels,format=info.format,byte_identical=False)
  if handle:
   try:
    assert(info.frames,info.samplerate,info.channels)==(c['frames'],c['rate'],c['channels']);data=(ctypes.c_float*(info.frames*info.channels))();n=lib.sf_readf_float(handle,data,info.frames);raw=b''.join(struct.pack('<f',x)for x in data);entry.update(read_frames=n,decoded_sha256=hashlib.sha256(raw).hexdigest(),byte_identical=n==info.frames and raw==expected)
   finally:lib.sf_close(handle)
  else:entry['error']=lib.sf_strerror(None).decode()
  results.append(entry)
 report=dict(version=lib.sf_version_string().decode(),cases=results);Path(a.output).write_text(json.dumps(report,indent=2)+'\n');print(report['version'],sum(x['byte_identical']for x in results),'/',len(results));raise SystemExit(0 if all(x['byte_identical']for x in results)else 1)
