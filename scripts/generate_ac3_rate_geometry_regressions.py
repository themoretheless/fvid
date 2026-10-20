"""Mutate only authored container rate metadata, retaining clean AC3 payloads."""
import json,struct,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'tests/fixtures/playback-errors/ac3-container-priming'
OUT=ROOT/'tests/fixtures/playback-errors/ac3-rate-geometry'
def generate():
    mkv=(BASE/'tone.mkv').read_bytes();mp4=bytearray((BASE/'tone.mp4').read_bytes())
    offsets=[at for at in range(len(mp4)-20) if mp4[at:at+4]==b'elst' and mp4[at+4:at+8]==bytes(4) and struct.unpack_from('>I',mp4,at+8)[0]==1 and struct.unpack_from('>i',mp4,at+16)[0]==256]
    assert len(offsets)==1;at=offsets[0]
    struct.pack_into('>i',mp4,at+16,0);mp4=bytes(mp4)
    old=b'\xb5\x88'+struct.pack('>d',48000.);new=b'\xb5\x88'+struct.pack('>d',32000.);assert mkv.count(old)==1
    at=mp4.index(b'ac-3')+28;assert struct.unpack_from('>I',mp4,at)[0]==48000<<16
    bad_mp4=bytearray(mp4);struct.pack_into('>I',bad_mp4,at,32000<<16)
    files={'control.mkv':mkv,'control.mp4':mp4,'mismatch.mkv':mkv.replace(old,new),'mismatch.mp4':bytes(bad_mp4)}
    for name,source,width in [('48k','ac3-stereo.ac3',768),('32k','ac3-mono-32k.ac3',576)]:files[name+'.ac3']=(ROOT/'tests/fixtures/audio'/source).read_bytes()[:width]
    damaged=bytearray(files['48k.ac3']);damaged[4]^=0x80;files['damaged-rate.ac3']=bytes(damaged)
    files['rate-change.ac3']=files['48k.ac3']+files['32k.ac3']
    files['manifest.json']=(json.dumps(dict(provenance='Existing authored120ms VP9/AC3 source containers; MP4 control edit begins at0so both rates stay sample aligned. Only SamplingFrequency/sample-entry rate changes48000to32000, coded CRC-protected AC3 remains identical. Clean32k/48k first-frame decoder controls read from authored tone fixtures. No private input/network/encoder.',sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
