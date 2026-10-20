#!/usr/bin/env python3
"""Compare two statically linked native decoder probe builds; never edits checkout.

Compile benchmark-mbaff-pcm-memory.rs against each compared fvid-codecs rlib:
rustc --edition=2024 -O <probe.rs> --extern fvid_codecs=<decoder.rlib> \
  -L dependency=<decoder dependency directory> -o <probe binary>
Then pass the before and after probe paths to this script.
The library profile affects diagnostic time; acceptance uses allocator requests.
"""
import argparse,datetime,hashlib,json,platform,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before',type=Path);parser.add_argument('after',type=Path)
    parser.add_argument('--max-side',type=int,choices=[64,128,256],default=256)
    args=parser.parse_args();runs=[]
    binaries={v:getattr(args,v).resolve() for v in ['before','after']}
    fixture=ROOT/'tests/fixtures/playback-errors/avc-mbaff-pcm-memory'
    for pair in range(6):
        for version in (['before','after'] if pair%2==0 else ['after','before']):
            output=subprocess.check_output([str(binaries[version]),str(fixture),str(args.max_side)],text=True)
            runs.append({'pair':pair+1,'version':version,'cases':[json.loads(line) for line in output.splitlines()]})
    summaries=[]
    for index in range(len(runs[0]['cases'])):
        samples={v:[r['cases'][index] for r in runs if r['version']==v] for v in ['before','after']}
        before,after=samples['before'][0],samples['after'][0]
        for version in samples:
            for metric in ['allocations','allocated_bytes','peak_requested_live_delta']:
                assert len({s[metric] for s in samples[version]})==1,'allocator comparison is not reproducible'
        side=after['side'];blocks=side*side//256
        view=(3*side*side+side*side//16)//(2 if after['field'] else 1)
        assert before['allocations']-after['allocations']==blocks*4
        assert before['allocated_bytes']-after['allocated_bytes']==blocks*view
        assert before['peak_requested_live_delta']-after['peak_requested_live_delta']==view
        summaries.append({'side':side,'field':after['field'],'saved_allocations':blocks*4,'saved_requested_bytes_per_decode':blocks*view,'saved_peak_requested_bytes':view})
    print(json.dumps({'checked_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'platform':platform.platform(),'binary_sha256':{v:hashlib.sha256(p.read_bytes()).hexdigest() for v,p in binaries.items()},'scope':'Allocator requests and peak requested live storage, not RSS. Debug-library elapsed times are diagnostic only.','summaries':summaries,'runs':runs},indent=2))
if __name__=='__main__':main()
