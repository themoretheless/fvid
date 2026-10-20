#!/usr/bin/env python3
"""Check improvement accounting; this infrastructure never adds accepted IDs."""
import json
from pathlib import Path


def main():
    root = Path(__file__).resolve().parents[1]
    directory = root / 'docs/improvements'
    registry = json.loads((directory / 'registry.json').read_text())
    entries = registry['entries']
    expected_ids = [f'FVID-{number:04d}' for number in range(1, len(entries)+1)]
    assert [entry['id'] for entry in entries] == expected_ids, 'IDs must be unique and sequential'
    accepted = [entry for entry in entries if entry['status'] == 'accepted']
    assert registry['accepted_count'] == len(accepted), 'accepted_count disagrees with accepted entries'
    assert len(accepted) <= registry['target'], 'accepted count exceeds the goal'
    assert registry['next_id'] == f'FVID-{len(entries)+1:04d}', 'next_id must name the next unused ID'
    for entry in entries:
        assert entry['status'] in ('accepted', 'pending'), f"unknown status: {entry['id']}"
        for field in ('before', 'result', 'title'):
            assert isinstance(entry[field], str) and entry[field].strip(), f"missing {field}: {entry['id']}"
        for field in ('source', 'regression'):
            path = (root / entry[field]).resolve()
            assert path.is_relative_to(root) and path.is_file(), f"missing {field}: {entry['id']}"
        if entry['status'] == 'accepted':
            path = (directory / entry['validation']).resolve()
            assert path.is_relative_to(directory) and path.is_file(), f"missing acceptance evidence: {entry['id']}"
            evidence = json.loads(path.read_text())
            if 'accepted_ids' in evidence:
                assert entry['id'] in evidence['accepted_ids'], f"ID absent from acceptance evidence: {entry['id']}"
            assert evidence.get('status', 'accepted') == 'accepted', f"evidence still pending: {entry['id']}"
    print(f"{len(accepted)} / {registry['target']} accepted; {len(entries)-len(accepted)} pending; accounting valid")


if __name__ == '__main__':
    main()
