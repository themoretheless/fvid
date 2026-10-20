"""Own bypass-coded FL wedge index component streams; no FFmpeg/network."""
import json
from pathlib import Path
import generate_hevc_wedge_tables as tables
OUT=tables.OUT
def generate():
    cases=json.loads(tables.generate()['manifest.json'])['cases'];files={}
    for c in cases:
        width=(c['count']-1).bit_length()
        for invalid in [False,True]:
            indices=[c['count']]if invalid else list(range(c['count']))
            low=0;bits=9
            for index in indices:
                for shift in reversed(range(width)):
                    low=2*low+(510 if index&(1<<shift)else 0);bits+=1
            code=low+509;padding=(-bits)%8
            name=f"{c['size']}-{'invalid'if invalid else 'indices'}.cabac"
            files[name]=(code<<padding).to_bytes((bits+padding)//8,'big')
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
