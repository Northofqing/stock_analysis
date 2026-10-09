import importlib.util
import json
import os
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('delivery', Path(__file__).resolve().parents[1] / 'prepare_goal_first_delivery.py')
d = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(d)


class FixedDelivery(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.runtime = self.root / 'runtime'
        self.runtime.mkdir(mode=0o700)
        self.patch = patch.object(d, 'RUNTIME', self.runtime)
        self.patch.start()
        self.bundle = self.root / 'candidate'
        self.bundle.mkdir(mode=0o700)
        for n in ('bin', 'scripts', 'resources', 'launchd'):
            (self.bundle / n).mkdir(mode=0o700)
        self.destination = self.runtime / 'tools/goal-first/v1'
        for name, mode in d.layout().items():
            data = b'fixture'
            if name == 'scripts/run-weekly-outcome-review.sh':
                data = d.launcher(self.destination).encode()
            elif name.startswith('launchd/'):
                data = plistlib.dumps(d.job(Path(name).stem, self.destination))
            d.write(self.bundle / name, data, mode)
        self.m = {'schema': 'goal-first-tools-package/v1', 'version': 'v1', 'runtime': str(self.runtime), 'destination': str(self.destination), 'launchd_labels': list(d.LABELS), 'preconditions': {'fixture': 'old'}, 'files': {name: {**d.identity(d.read(self.bundle / name, True)), 'mode': oct(mode)} for name, mode in d.layout().items()}}
        d.write(self.bundle / 'manifest.json', d.json_bytes(self.m))
        self.base = patch.object(d, 'baseline', return_value=self.m['preconditions'])
        self.base.start()

    def tearDown(self):
        self.base.stop()
        self.patch.stop()
        self.tmp.cleanup()

    def test_default_check_no_write_and_exact_version_job_paths(self):
        before = {str(p): d.read(p) for p in self.bundle.rglob('*') if p.is_file()}
        d.check(self.bundle)
        self.assertFalse(self.destination.exists())
        self.assertEqual(before, {str(p): d.read(p) for p in self.bundle.rglob('*') if p.is_file()})
        for label in d.LABELS:
            job = d.job(label, self.destination)
            self.assertFalse(job['RunAtLoad'])
            self.assertFalse(job['KeepAlive'])
            self.assertTrue(any(str(self.destination) in x for x in job['ProgramArguments']))

    def test_private_resource_symlink_hardlink_and_extra_refused(self):
        resource = self.bundle / 'resources/signal_registry.toml'
        resource.chmod(0o644)
        with self.assertRaises(ValueError): d.validate(self.bundle)
        resource.chmod(0o400)
        os.link(resource, self.root / 'alias')
        with self.assertRaises(ValueError): d.validate(self.bundle)
        (self.root / 'alias').unlink()
        resource.unlink()
        resource.symlink_to(self.bundle / 'resources/a_share_market_holidays.csv')
        with self.assertRaises(ValueError): d.validate(self.bundle)

    def test_drift_and_foreign_destination_refused(self):
        with patch.object(d, 'baseline', return_value={'fixture': 'changed'}):
            with self.assertRaises(ValueError): d.install(self.bundle)
        self.assertFalse(self.destination.exists())
        self.m['destination'] = str(self.root / 'foreign')
        (self.bundle / 'manifest.json').write_bytes(d.json_bytes(self.m))
        with self.assertRaises(ValueError): d.validate(self.bundle)

    def test_atomic_publish_idempotent_nooverwrite_and_evidence_retained(self):
        self.assertEqual(d.install(self.bundle)['status'], 'published')
        self.assertEqual(d.install(self.bundle)['status'], 'already_published')
        self.assertEqual(d.read(self.destination / 'manifest.json'), d.read(self.bundle / 'manifest.json'))
        extra = self.destination / 'unlisted'
        d.write(extra, b'local evidence')
        with self.assertRaises(ValueError): d.install(self.bundle)
        self.assertEqual(extra.read_bytes(), b'local evidence')

    def test_fixed_third_weekly_provenance_pins_full_reviewed_blobs(self):
        import subprocess
        repo = Path(__file__).resolve().parents[2]
        original = subprocess.check_output(['git', 'show', '1ce18c2850d19b4bd0908bce6225049c61b0d19d:src/bin/weekly_outcome_review/registry.rs'], cwd=repo)
        reviewed = (repo / 'src/bin/weekly_outcome_review/registry.rs').read_bytes()
        current = reviewed
        def git(args, **kwargs):
            if args[1] == 'diff':
                if args[3] == 'weekly': return ''
                return 'src/bin/assistant_review.rs\n' if args[4] == 'assistant' else 'src/bin/weekly_outcome_review/registry.rs\n'
            return original if args[2].startswith('assistant:') else current
        with patch.object(d.subprocess, 'check_output', side_effect=git):
            sources = d.binary_sources(self.root, 'fixed', 'base', 'assistant', self.root / 'log', 'weekly')
            self.assertEqual(sources['weekly_outcome_review'], 'weekly')
            self.assertEqual(sources['assistant_review'], 'assistant')
            self.assertEqual(sources['sell_reminder_preview'], 'base')
            with self.assertRaises(ValueError): d.binary_sources(self.root, 'fixed', 'base', 'assistant')
            # Both were admitted by the former protected-span comparison.
            shadow = b'\nmod toml { pub fn from_str<T: serde::de::DeserializeOwned>(_: &str) -> Result<T, ::toml::de::Error> { ::toml::from_str(super::DEFAULT_REGISTRY) } }\n'
            for current in (reviewed + shadow, reviewed.replace(b'fn same_registry_file', shadow + b'fn same_registry_file'), reviewed.replace(b'toml::from_str(raw)', b'toml::from_str(DEFAULT_REGISTRY)')):
                with self.assertRaisesRegex(ValueError, 'exact reviewed registry blobs'):
                    d.binary_sources(self.root, 'fixed', 'base', 'assistant', self.root / 'log', 'weekly')
            current = reviewed
            def changed_after_build(args, **kwargs):
                return 'src/assistant_review.rs\n' if args[1] == 'diff' and args[3] == 'weekly' else git(args, **kwargs)
            with patch.object(d.subprocess, 'check_output', side_effect=changed_after_build):
                with self.assertRaisesRegex(ValueError, 'compiled inputs changed after weekly build'):
                    d.binary_sources(self.root, 'fixed', 'base', 'assistant', self.root / 'log', 'weekly')
            original += b'\n// unreviewed old source'
            with self.assertRaisesRegex(ValueError, 'exact reviewed registry blobs'):
                d.binary_sources(self.root, 'fixed', 'base', 'assistant', self.root / 'log', 'weekly')
        with patch.object(d.subprocess, 'check_output', return_value='src/llm/bounded.rs\n'):
            with self.assertRaises(ValueError): d.binary_sources(self.root, 'fixed', 'base', 'assistant', self.root / 'log', 'weekly')
        with patch.object(d.subprocess, 'check_output', return_value='src/bin/assistant_review.rs\n'):
            self.assertEqual(d.binary_sources(self.root, 'assistant', 'base')['weekly_outcome_review'], 'base')

    def test_hash_mismatch_and_escaped_version_refused(self):
        target = self.bundle / 'scripts/reliability_common.py'
        target.write_bytes(b'changed')
        with self.assertRaises(ValueError): d.validate(self.bundle)
        for version in ('../escape', '.', '..', 'x' * 65):
            with self.assertRaises(ValueError): d.prepare(self.root, self.root, version, 'x', self.root / 'log')
        self.m['version'] = '.'
        (self.bundle / 'manifest.json').write_bytes(d.json_bytes(self.m))
        with self.assertRaises(ValueError): d.validate(self.bundle)
        self.assertFalse((self.root.parent / 'escape').exists())


if __name__ == '__main__': unittest.main()
