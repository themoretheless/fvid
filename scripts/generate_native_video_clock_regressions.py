"""Re-clock twelve saved authored AVC pictures; no external encoder or media."""
from pathlib import Path
import hashlib, json, struct
from generate_playback_error_samples import atom, children
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/native-video-clock'
def rewrite(data):
    out=b''
    for kind,body in children(data):
        if kind==b'edts': continue
        if kind in {b'moov',b'trak',b'mdia',b'minf',b'stbl'}: body=rewrite(body)
        elif kind==b'mdhd':
            # Version one permits a duration beyond the u64 nanosecond horizon.
            body=b'\1\0\0\0'+bytes(16)+struct.pack('>IQ',7,140_000_000_007)+body[20:24]
        elif kind==b'stts': body=bytes(4)+struct.pack('>III',1,12,2_000_000_001)
        out+=atom(kind,body)
    return out
def generate():
    source=OUT.parent/'control.mp4'
    files={'clock.mp4':rewrite(source.read_bytes())}
    files['manifest.json']=(json.dumps({'provenance':'Twelve saved authored AVC pictures from control.mp4. Long synthetic clock only; no private media, encoder or network.','timescale':7,'sample_delta':2_000_000_001,'declared_duration_ticks':140_000_000_007,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items(): (OUT/name).write_bytes(data)
