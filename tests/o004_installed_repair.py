"""Native regression: installed repair must not depend on extracted release files.

Uses a new isolated HOME/data root; never changes the caller's installation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    work = Path(tempfile.mkdtemp(prefix='axiom-o004-repair-'))
    release = work / 'release'
    release.mkdir()
    with tarfile.open(args.archive) as archive:
        archive.extractall(release, filter='data')
    shutil.copy2(args.cli, release / 'axiom-cli')
    env = {k: v for k, v in os.environ.items() if not k.startswith('AXIOM_')}
    env.update(HOME=str(work / 'home'), XDG_DATA_HOME=str(work / 'data'), SHELL='/bin/zsh')
    Path(env['HOME']).mkdir()
    root = work / 'data/axiom'
    cases = []

    def run(name, argv, expected=0, check=None):
        proc = subprocess.run([str(x) for x in argv], env=env, capture_output=True, text=True, timeout=300)
        combined = proc.stdout + proc.stderr
        passed = proc.returncode == expected and (check is None or check(proc))
        cases.append({'case': name, 'exit_code': proc.returncode, 'expected_exit_code': expected,
                      'passed': passed, 'output': combined.replace(str(work), '<isolated-root>')})
        if not passed:
            raise AssertionError(f'{name}: {combined}')
        return proc

    try:
        run('fresh_install', [release / 'axiom-cli', 'install', '--yes', '--no-modify-path', '--json'])
        shutil.rmtree(release)
        shutil.rmtree(root / 'staging')
        cli = root / 'bin/axm'
        result = run('repair_without_archive_or_staging', [cli, 'install', '--yes', '--no-modify-path', '--json'])
        assert 'installed_bundle' in result.stdout
        run('compact_doctor', [cli, 'doctor'], check=lambda p: len(p.stdout.splitlines()) <= 5
            and 'Axiom installation: OK' in p.stdout and 'sha256' not in p.stdout)
        run('compact_version_includes_actual_skills', [cli, 'version'], check=lambda p: 'Skills: 0.1.5' in p.stdout and 'not recorded' not in p.stdout)
        run('verbose_doctor', [cli, 'doctor', '--verbose'], check=lambda p: 'sha256=' in p.stdout)
        run('json_doctor', [cli, 'doctor', '--json'], check=lambda p: 'findings' in json.loads(p.stdout)['details'])
        skills = root / 'installs/ecosystem/skills'
        pointer = json.loads((skills / 'current').read_text())
        payload = skills / pointer['version'] / 'policy/POLICY.md'
        original = payload.read_bytes()
        record = (root / 'installed.json').read_bytes()
        payload.write_bytes(original + b'\nchanged\n')
        run('tampered_skills_refused', [cli, 'install', '--yes', '--no-modify-path', '--json'], expected=2)
        assert (root / 'installed.json').read_bytes() == record
        payload.write_bytes(original)
        pointer_path = skills / 'current'
        original_pointer = pointer_path.read_bytes()
        pointer['manifest_sha256'] = '0' * 64
        pointer_path.write_text(json.dumps(pointer))
        run('tampered_manifest_pointer_refused', [cli, 'install', '--yes', '--no-modify-path', '--json'], expected=2)
        assert (root / 'installed.json').read_bytes() == record
        run('version_refuses_tampered_skills_metadata', [cli, 'version', '--json'], expected=2)
        pointer_path.write_bytes(original_pointer)
        pointer_path.rename(skills / 'linked-current-target')
        pointer_path.symlink_to(skills / 'linked-current-target')
        run('linked_skills_pointer_refused', [cli, 'install', '--yes', '--no-modify-path', '--json'], expected=2)
        assert (root / 'installed.json').read_bytes() == record
        pointer_path.unlink()
        (skills / 'linked-current-target').rename(pointer_path)
        pointer_path.rename(skills / 'saved-current')
        run('missing_skills_refused', [cli, 'install', '--yes', '--no-modify-path', '--json'], expected=4)
        assert (root / 'installed.json').read_bytes() == record
        (skills / 'saved-current').rename(pointer_path)
        run('repair_after_restoring_skills', [cli, 'install', '--yes', '--no-modify-path', '--json'])
    finally:
        args.report.write_text(json.dumps({'native_platform': os.uname().sysname, 'machine': os.uname().machine,
            'archive_sha256': hashlib.sha256(args.archive.read_bytes()).hexdigest(),
            'cli_sha256': hashlib.sha256(args.cli.read_bytes()).hexdigest(), 'cases': cases}, indent=2) + '\n')
    print(f'Passed {len(cases)} native installed-repair cases')


if __name__ == '__main__':
    main()
