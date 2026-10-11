# M-001 — Mac Intel bootstrap verification

The canonical card is in axiom-specs at `tasks/M/M-001.md` (ADR-0034).

`python tests/l004_posix_bootstrap.py --archive <native-macos-x64.tar.gz> --work <new-directory>` runs options, no-terminal, digest and network boundaries with isolated homes. Root identity is checked by `python -m unittest tests.test_bootstrappers -v`; hosted Mac CI also executes only the root refusal with `sudo -n`. No elevated install occurs.

`tests/l007_fresh_runner.py` consumes the native CI candidate on a separate fresh Mac Intel runner. `tests/m001_published_oneliner.py --out <report.json>` requires a fresh hosted account and runs the actual published one-liner with a native PTY. `--work <new-directory>` permits local debugging while recording `clean_user_account: false`. Published and candidate evidence remain distinct.

The `.github/workflows/distribution-ci.yml` artifacts retain command outputs, source revision, run URL and asset SHA-256 values. M-002 must independently assess the published proof after M-001 is done. No new release, tag, contract or engine behavior is introduced.
