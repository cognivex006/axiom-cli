"""POSIX installers verify structured identity independently of human wording."""
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


class RuntimeVersionProtocol(unittest.TestCase):
    def runtimes(self):
        for target in ['linux', 'macos']:
            spec = importlib.util.spec_from_file_location(
                'runtime_protocol_' + target,
                ROOT / 'packaging' / target / 'Provision-McpRuntime.py',
            )
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            yield target, module

    def test_accepts_structured_identity_and_requests_json(self):
        for target, runtime in self.runtimes():
            with self.subTest(target=target):
                body = dict(component='axiom-mcp', version='0.1.13', build_revision='a' * 40)
                with patch.object(runtime, 'command', return_value=json.dumps(body)) as command:
                    runtime.verify_entrypoint(Path('/private/axmp'), {'PATH': '/private'}, '0.1.13', 'a' * 40)
                command.assert_called_once_with(
                    [str(Path('/private/axmp')), 'version', '--json'],
                    {'PATH': '/private', 'AXIOM_MCP_BUILD_REVISION': 'a' * 40},
                )

    def test_rejects_wrong_identity_and_non_protocol_output(self):
        valid = dict(component='axiom-mcp', version='0.1.13', build_revision='a' * 40)
        bad = [json.dumps({**valid, key: value}) for key, value in [
            ('version', '0.1.130'), ('version', 13), ('build_revision', 'a' * 40 + 'suffix'),
            ('build_revision', 'b' * 40), ('component', 'axiom-cli'),
        ]]
        bad += ['Axiom MCP 0.1.13', 'version: 0.1.13\nbuild_revision: ' + 'a' * 40,
                'banner\n' + json.dumps(valid), '[]', 'null', '{}']
        for target, runtime in self.runtimes():
            for output in bad:
                with self.subTest(target=target, output=output):
                    with patch.object(runtime, 'command', return_value=output):
                        with self.assertRaises(ValueError):
                            runtime.verify_entrypoint(Path('/private/axmp'), {}, '0.1.13', 'a' * 40)


if __name__ == '__main__':
    unittest.main()
