"""Own changing AAC with a finite edit beyond the indexed source extent."""
from pathlib import Path
import json,hashlib,struct
from generate_mp4_aac_transitions import generate as controls,SCALE
from generate_mp4_variable_rate_edits import edit_movie
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-aac-native-grid-missing-extent-seek'
def generate():
    source=controls()['changes.mp4'];movie=dict(atoms(source));track=next(b for k,b in atoms(movie[b'moov'])if k==b'trak'and b'soun'in b);mdia=dict(atoms(track));tables=dict(atoms(dict(atoms(dict(atoms(mdia[b'mdia']))[b'minf']))[b'stbl']));end=sum(n*d for n,d in struct.iter_unpack('>II',tables[b'stts'][8:]));name='past-source-aac.mp4';data=edit_movie(source,[(1,end+SCALE//100)]);return{name:data,'manifest.json':(json.dumps(dict(provenance='Own LC/Main AAC pulses and neutral VP9. One1ms edit starts10ms beyond the coded endpoint;complete packet bodies and native-grid timing. No private input,FFmpeg or network.',sha256={name:hashlib.sha256(data).hexdigest()}),indent=2)+'\n').encode()}
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
