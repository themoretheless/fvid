"""Owned multiplexed E-AC-3 base-programme routing witnesses."""
import json
from pathlib import Path
from generate_eac3_zero_bit import packet
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-substreams'

def extra(kind,identity,mapped=False):
 # Author dependent syntax directly, including compr, channel map, and the
 # absence of independent-only converter fields in the header and blocks.
 return packet(False,3,8,[(1,1)]*6,False,stream_type=kind,substream_id=identity,channel_map=0xa000 if mapped else None)

def generate():
 files={};cases=[]
 modes={f'ind{i}':[(0,j,False)for j in range(1,i+1)]for i in range(1,8)}
 modes.update(dep=[(1,0,False)],depmap=[(1,0,True)],programmes=[(0,i,False)for i in range(1,8)],mixed=[(1,0,True)]+[(0,i,False)for i in range(1,8)]+[(1,0,False)])
 for allocated in [False,True]:
  for converted in [False,True]:
   for mask in [0,3]:
    base=packet(converted,mask,8,[(1,1)]*6,allocated)
    for mode,streams in modes.items():
     name=f'a{int(allocated)}-t{2 if converted else 0}-m{mask}-{mode}'
     added=b''.join(extra(*s)for s in streams);bundle=base+added
     files[name+'.ec3']=bundle*3;files[name+'-control.ec3']=base*3
     files[name+'.mkv']=movie([bundle]*3);files[name+'-control.mkv']=movie([base]*3)
     files[name+'-extra.ec3']=added
     cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',extra=name+'-extra.ec3',streams=streams))
 example=cases[0];tail=files[example['file']][:-1]
 files['truncated-tail.ec3']=tail
 packet_size=len(files[example['file']])//3
 packets=[tail[:packet_size],tail[packet_size:2*packet_size],tail[2*packet_size:]]
 files['truncated-tail.mkv']=movie(packets)
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files

if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
