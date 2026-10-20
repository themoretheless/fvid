#!/usr/bin/env python3
"""Replay paired CAS mask8 measurements without editing the checkout."""
import hashlib,json,platform,statistics,subprocess,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
CAS=ROOT/'crates/fvid-media/src/owned_cas.rs'
old='if self.planes == 0'
new='if !planes.iter().any(|plane| self.planes & plane.mask != 0)'
def main():
    after=CAS.read_text()
    assert after.count(new)==1,'expected optimized plane admission predicate'
    before=after.replace(new,old)
    template=Path(__file__).with_suffix('.rs').read_text()
    runs=[]
    with tempfile.TemporaryDirectory(prefix='fvid-cas-paired-') as directory:
        work=Path(directory);binaries={}
        for version,source in [('before',before),('after',after)]:
            kernel=work/(version+'-cas.rs');kernel.write_text(source)
            benchmark=template
            for module in ['owned_cas','owned_frame','owned_expression']:
                location=kernel if module=='owned_cas' else ROOT/'crates/fvid-media/src'/f'{module}.rs'
                benchmark=benchmark.replace(f'../../crates/fvid-media/src/{module}.rs',str(location))
            sourcefile=work/(version+'.rs');sourcefile.write_text(benchmark)
            binary=work/version;subprocess.run(['rustc','--edition=2024','-O',str(sourcefile),'-o',str(binary)],check=True);binaries[version]=binary
        for pair in range(6):
            for version in (['before','after'] if pair%2==0 else ['after','before']):
                output=subprocess.check_output([str(binaries[version])],text=True)
                runs.append({'pair':pair+1,'version':version,'cases':[json.loads(line) for line in output.splitlines()]})
    summaries=[]
    for index in range(3):
        case=runs[0]['cases'][index];summary={key:case[key] for key in ['width','height','depth']}
        for version in ['before','after']:
            samples=[r['cases'][index] for r in runs if r['version']==version]
            summary[version+'_median_ns_per_call']=statistics.median(s['elapsed_ns']/s['calls'] for s in samples)
            summary[version+'_allocations_per_call']=statistics.median(s['allocations']/s['calls'] for s in samples)
            summary[version+'_allocated_bytes_per_call']=statistics.median(s['allocated_bytes']/s['calls'] for s in samples)
        summaries.append(summary)
    print(json.dumps({'platform':platform.platform(),'source_hashes':{v:hashlib.sha256(s.encode()).hexdigest() for v,s in [('before',before),('after',after)]},'scope':'Optimized actual CAS kernel, three-channel RGB and alpha-only mask; not full playback throughput or RSS.','summaries':summaries,'runs':runs},indent=2))
if __name__=='__main__':main()
