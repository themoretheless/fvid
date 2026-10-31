"""Archive completed qualification, verify frozen sources, and count unique checks."""
from pathlib import Path
import hashlib
import json
import re

q = Path(__file__).resolve().parent
results = json.loads((q / 'qualification-results.json').read_text())
plan = json.loads((q / 'qualification-plan.json').read_text())
assert results.keys() == plan['commands'].keys()
assert all(v['exit_code'] == 0 and v['failed'] == 0 for v in results.values())
sources = json.loads((q / 'after-source.json').read_text())
for name, digest in sources.items():
    assert hashlib.sha256(Path(name).read_bytes()).hexdigest() == digest, name
logs = q / 'qualification'
logs.mkdir(exist_ok=True)
manifest = {}
passed = {}
ignored = {}
for kind, result in results.items():
    original = Path(result['log'])
    raw = original.read_bytes()
    text = raw.decode(errors='replace')
    archive = logs / (kind + '.log')
    archive.write_text('\n'.join(line.rstrip() for line in text.splitlines()).rstrip() + '\n')
    manifest[kind] = {'raw_sha256': hashlib.sha256(raw).hexdigest(),
                      'archived_sha256': hashlib.sha256(archive.read_bytes()).hexdigest(),
                      'archive': str(archive.relative_to(q))}
    passed[kind] = re.findall(r'^test (\S+) \.\.\. ok$', text, re.M)
    ignored[kind] = re.findall(r'^test (\S+) \.\.\. ignored.*$', text, re.M)
    assert len(passed[kind]) == result['passed'], (kind, len(passed[kind]), result)
root_groups = ['root-mp4', 'root-webm', 'player-units', 'audio-thread']
root_names = [name for kind in root_groups for name in passed[kind]]
root_union = set(root_names)
# These three root/native_media tests compile the same shared implementation as
# fvid-media's owned_mp4_audio module. Keep one execution per physical test.
shared = [
    'comparing_and_scaling_large_fractions_does_not_multiply_denominators',
    'normalization_origins_keep_signed_fractional_output_phase',
    'source_phase_and_signed_preroll_survive_fractional_container_clocks',
]
for name in shared:
    assert 'native_media::mp4_export_clock::tests::' + name in root_union
    assert 'owned_mp4_audio::mp4_export_clock::tests::' + name in passed['media']
report = {'root_unique_passed': len(root_union),
          'root_duplicate_events': len(root_names) - len(root_union),
          'root_overlap_names': sorted(n for n in root_union if root_names.count(n) > 1),
          'shared_root_media_tests': shared,
          'distinct_passed': results['integration']['passed'] + len(root_union) + results['media']['passed'] - len(shared),
          'ignored_by_gate': ignored,
          'note': 'Focused checks and 20 repeated runs are supporting evidence, not additional distinct checks or improvement IDs.'}
(q / 'qualification-log-hashes.json').write_text(json.dumps(manifest, indent=2) + '\n')
(q / 'deduplication.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
