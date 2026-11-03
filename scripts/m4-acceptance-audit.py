from pathlib import Path
import hashlib
import json
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / '.refs/m4-acceptance'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(path):
    rows = json.loads(path.read_text(encoding='utf-8-sig'))
    changed = []
    for row in rows:
        source = Path(row['Path'])
        if not source.exists() or sha(source) != row['Hash'].lower():
            changed.append(str(source.relative_to(ROOT)))
    return dict(record=str(path.relative_to(ROOT)), sha256=sha(path), entries=len(rows),
                changed_since_compilation=changed, all_match=not changed)


def main():
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    target = EVIDENCE / 'initial-audit.json'
    if target.exists():
        raise SystemExit('Preserve the initial audit; choose a fresh evidence directory for another audit.')
    records = [
        '.refs/m4-progress/bin/progress-final.sources.json',
        '.refs/m4-startup/bin/candidate-dev.sources.json',
        '.refs/m4-history/bin/candidate-normal.sources.json',
        '.refs/m4-history/bin/restored-normal.sources.json',
        '.refs/m4-limits/bin/normal-limits.sources.json',
        '.refs/m4-limits/bin/final-normal.sources.json',
        'measurements/m4-limits/root-check/source-hashes.json',
    ]
    files = [
        '.refs/m4-progress/handoff.md', '.refs/m4-progress/final-results.json',
        '.refs/m4-progress/identities.json', '.refs/m4-progress/expected-rejections.json',
        '.refs/m4-startup/handoff.md', '.refs/m4-startup/identities.json',
        '.refs/m4-history/handoff.md', '.refs/m4-history/identities.json',
        '.refs/m4-history/final-summary.json', '.refs/m4-startup-history-review.md',
        '.refs/m4-limits/handoff.md', '.refs/m4-limits/summary.json',
        '.refs/m4-limits/registered-defaults.json',
        '.refs/m4-progress/bin/progress-final.exe',
        '.refs/m4-history/bin/candidate-normal.exe',
        '.refs/m4-limits/bin/normal-limits.exe',
        '.refs/m4-limits/bin/final-normal.exe',
        'measurements/m4-limits/root-check/result.txt',
        'measurements/m4-limits/root-check/gpu.stdout.log',
        'measurements/m4-limits/root-check/gpu.stderr.log',
        'measurements/m4-limits/root-check/test-binary-hashes.json',
        'measurements/m4-cook/registration-initial/cameras.json',
        'measurements/m4-cook/root-unit-a/material-thin.manifest',
        'measurements/m4-cook/root-unit-a/closed-sphere.manifest',
    ]
    result = dict(head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  inventories=[inventory(ROOT / p) if (ROOT / p).exists() else dict(record=p, missing=True) for p in records],
                  artifacts={p: sha(ROOT / p) if (ROOT / p).exists() else None for p in files})
    target.write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


def final_audit():
    initial = json.loads((EVIDENCE / 'initial-audit.json').read_text())
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    assert head == initial['head']
    compiled = {}
    validation_flags = b'''        flags: if o.validate {
            wgpu::InstanceFlags::debugging()
        } else {
            wgpu::InstanceFlags::default()
        },
'''
    for label in ['acceptance-dev', 'acceptance-release', 'restored-normal']:
        record = EVIDENCE / f'bin/{label}.sources.json'
        records = json.loads(record.read_text(encoding='utf-8-sig'))
        for row in records:
            path = Path(row['Path'])
            data = path.read_bytes()
            if label == 'acceptance-dev' and path == ROOT / 'examples/virtual-stream.rs':
                assert data.count(validation_flags) == 1
                data = data.replace(validation_flags, b'')
            assert hashlib.sha256(data).hexdigest() == row['Hash'].lower(), (label, path)
            target = EVIDENCE / 'compiled' / label / path.relative_to(ROOT)
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists():
                assert target.read_bytes() == data
            else:
                target.write_bytes(data)
        compiled[label] = dict(source_record_sha256=sha(record), entries=len(records),
                               executable_sha256=sha(EVIDENCE / f'bin/{label}.exe'))
    root = inventory(ROOT / 'measurements/m4-limits/root-check/source-hashes.json')
    assert set(p.replace('\\', '/') for p in root['changed_since_compilation']) == {
        'examples/virtual-stream.rs', 'examples/m4/diagnostics.rs'}
    historical = {}
    for folder, label in [('m4-progress', 'progress-final'), ('m4-startup', 'startup-final'),
                          ('m4-history', 'candidate-normal'), ('m4-limits', 'normal-limits')]:
        record = ROOT / f'.refs/{folder}/bin/{label}.sources.json'
        historical[f'{folder}/{label}'] = dict(executable_sha256=sha(record.with_name(label+'.exe')),
                                               source=inventory(record))
    runs = {}
    for run in sorted((ROOT / 'measurements/m4-acceptance').iterdir()):
        if not (run / 'process.json').exists():
            continue
        before = json.loads((run / 'sources-before.json').read_text(encoding='utf-8-sig'))
        after = json.loads((run / 'sources-after.json').read_text(encoding='utf-8-sig'))
        assert before == after, run
        source = json.loads((run / 'compiled-sources.json').read_text(encoding='utf-8-sig'))
        assert source == before, (run, 'runtime source differs from compiled inventory')
        identity = json.loads((run / 'identity.json').read_text(encoding='utf-8-sig'))
        for item in identity:
            assert sha(Path(item['Path'])) == item['Hash'].lower()
        runs[run.name] = dict(process_sha256=sha(run / 'process.json'), identity=identity,
                              source_record_sha256=sha(run / 'compiled-sources.json'))
    assert len(runs) == 28
    owned = ['examples/virtual-stream.rs', 'examples/m4/diagnostics.rs', 'docs/m4-acceptance-matrix.md']
    owned += [str(p.relative_to(ROOT)).replace('\\', '/') for p in sorted((ROOT / 'scripts').glob('m4-acceptance-*'))]
    artifacts = ['handoff.md', 'initial-audit.json', 'run-summary.json', 'timing-spread.json',
                 'image-metrics.json', 'quality-summary.json', 'scorer-identity.json',
                 'restored-binaries.json', 'final-state.json', 'contact-material-thin.png', 'contact-closed-sphere.png']
    result = dict(head=head, compiled=compiled, historical=historical, root_check_applicability=root, runs=runs,
                  owned_files={p: sha(ROOT / p) for p in owned},
                  artifacts={p: sha(EVIDENCE / p) for p in artifacts},
                  restored_viewer_sha256=sha(ROOT / 'target/debug/renderlab-viewer.exe'),
                  restored_replay_sha256=sha(ROOT / 'target/debug/examples/virtual-stream.exe'))
    (EVIDENCE / 'final-identity.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(dict(head=head, compiled=compiled, audited_runs=len(runs),
                          root_check_changed_sources=root['changed_since_compilation']), indent=2))


if __name__ == '__main__':
    if sys.argv[1:] == ['--final']:
        final_audit()
    elif not sys.argv[1:]:
        main()
    else:
        raise SystemExit('Usage: m4-acceptance-audit.py [--final]')
