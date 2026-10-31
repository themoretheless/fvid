#!/usr/bin/env python3
"""Preserve sampling protocol; memory task requires CPU non-regression, not another seek speedup."""
import contextlib
import io
import json
import sys
import compare_mp4_seek_navigation

def main():
    args = sys.argv[:]
    assert len(args) >= 4, 'baseline, after, workload required'
    if len(args) == 4:
        args.append('10000')
    if len(args) == 5:
        args.append('7')
    sys.argv = args
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        compare_mp4_seek_navigation.main()
    result = json.loads(out.getvalue())
    result['historical_seek_speedup_goal'] = {
        'criteria': result.pop('criteria'),
        'met': result.pop('accepted_by_criteria'),
        'note': 'Original CPU speedup criteria belong to FVID0442; they are not the memory-improvement acceptance condition.'
    }
    result['criteria'] = 'Memory task CPU guard fixed before timing: median after <=1.10*baseline for opening, navigation and actual seek+RGB; same input/source harness/profile,7 paired rounds.'
    result['cpu_nonregression'] = all(
        result['statistics']['after'][key]['median'] <= 1.10 * result['statistics']['baseline'][key]['median']
        for key in ['open_ns', 'navigation_ns', 'seek_rgb_ns']
    )
    print(json.dumps(result, indent=2))

if __name__ == '__main__':
    main()
