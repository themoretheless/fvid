#!/usr/bin/env python3
"""Explicit offline partition analysis; not an HRTF conversion/playback oracle."""
import cmath,hashlib,json,math,re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
FACTS=ROOT/'crates/fvid-media/src/owned_aac/aac_ld_mps_prepare_facts.rs'
GRIDS=[4,5,7,9,12,15,23]
def analyze():
    text=FACTS.read_text()
    def values(name):
        match=re.search(r'const '+name+r':[^=]+?=\s*\[([^]]+)\]',text,re.S)
        if not match:raise ValueError('missing numeric fact '+name)
        return [int(v.strip())for v in match[1].split(',')if v.strip()]
    maps={g:values('MAP'+str(g))for g in GRIDS}
    qmf={g:values('QMF'+str(g))for g in GRIDS}
    assert all(len(maps[g])==23 and len(qmf[g])==128 for g in GRIDS)
    equivalences=[];pairs=[];phase_differences=[]
    for bins in [32,64,128]:
        fine=qmf[23][:bins]
        for g in GRIDS:
            expanded=[maps[g][i]for i in fine]
            mismatch=[i for i in range(bins)if expanded[i]!=qmf[g][i]]
            assert not mismatch,(g,bins,mismatch)
            equivalences.append(dict(native_bands=g,qmf_bins=bins,mismatched_bins=mismatch))
            # Repeating native parameters on the finest grid does not preserve
            # an index-dependent branch automatically. This is a structural
            # counterexample, not evidence for either phase interpretation.
            differing=[i for i in range(bins)if (qmf[g][i]<12)!=(fine[i]<12)]
            phase_differences.append(dict(native_bands=g,qmf_bins=bins,differing_bins=differing))
        for spatial in GRIDS:
            for hrtf in GRIDS:
                native=list(zip(qmf[spatial][:bins],qmf[hrtf][:bins]))
                lifted=[(maps[spatial][i],maps[hrtf][i])for i in fine]
                assert native==lifted
                regions=[]
                for i,pair in enumerate(native):
                    if not regions or tuple(regions[-1]['pair'])!=pair:
                        regions.append(dict(first_qmf_bin=i,end_qmf_bin=i+1,pair=list(pair)))
                    else:regions[-1]['end_qmf_bin']=i+1
                assert len(regions)<=23
                pairs.append(dict(spatial_bands=spatial,hrtf_bands=hrtf,qmf_bins=bins,regions=regions))
    native_response=cmath.exp(1j*(4*math.pi/16)/2)
    phase_example=dict(spatial_bands=4,qmf_bin=12,native_index=qmf[4][12],fine_index=qmf[23][12],coded_center_level=16,coded_center_phase=4,native_response=[native_response.real,native_response.imag],lifted_index_response=[1.0,0.0],absolute_response_difference=abs(native_response-1),scope='Analytic branch witness only, not a waveform or normative phase-selection oracle.')
    return dict(phase_gate_example=phase_example,scope='Exact finite-table partition analysis only. No normative HRTF conversion, numerical matrix/PCM oracle, codec acceptance or conformance claim.',facts_path=str(FACTS.relative_to(ROOT)),facts_sha256=hashlib.sha256(FACTS.read_bytes()).hexdigest(),native_equivalences=equivalences,pair_partitions=pairs,phase_gate_counterexamples=phase_differences,conclusion='All21 native maps and147 joint partitions lift exactly through23 processing bands, with at most23 overlay regions. An index-dependent phase cutoff is not invariant under that lift. Matrix time histories, cutoffs, smoothing and in-band reconfiguration also require qualification before renderer changes.')
if __name__=='__main__':
    out=ROOT/'docs/validation/aac-ld-mps-hrtf-partitions-2026-11-01.json'
    report=analyze();out.write_text(json.dumps(report,separators=(',',':'))+'\n')
    print('Native map equivalences:',len(report['native_equivalences']))
    print('Exact joint partitions:',len(report['pair_partitions']))
    print('Phase counterexamples:',sum(bool(x['differing_bins'])for x in report['phase_gate_counterexamples']))
