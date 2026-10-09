import fcntl
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import rotate_push_log as r
from reliability_common import open_dir


class RotationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.runtime = Path(self.tmp.name).resolve()
        self.root = self.runtime / 'data/push_log'
        self.old = self.root / '2026-07-01'
        self.old.mkdir(parents=True)
        self.archive = self.runtime / 'archives'
        self.lock = self.root / '.push_log.lock'
        self.lock.touch(mode=0o600)
        self.original = {'a.md': b'\xff\x00\nexact\r\n', 'b.md': b'hello\n'}
        for name, raw in self.original.items():
            (self.old / name).write_bytes(raw)
        (self.old / 'counted.json').write_bytes(b'{}')
        for day in ('2026-10-09', '2026-10-10'):
            folder = self.root / day
            folder.mkdir()
            (folder / 'today.md').write_bytes(b'keep')
        self.inodes = {p: p.stat().st_ino for p in (self.root, self.old, self.lock, self.old / 'counted.json')}

    def tearDown(self):
        self.tmp.cleanup()

    def rotate(self, **kwargs):
        return r.rotate(self.runtime, self.archive, observed_at='2026-10-09T10:00:00+08:00', **kwargs)

    def test_default_dry_run_changes_nothing(self):
        before = {str(p): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        report = self.rotate()
        self.assertEqual(report['results'][0]['mode'], 'dry_run')
        self.assertFalse(self.archive.exists())
        self.assertEqual(before, {str(p): p.read_bytes() for p in self.root.rglob('*') if p.is_file()})

    def test_roundtrip_idempotence_and_explicit_prune_preserves_namespaces(self):
        self.rotate(archive=True)
        fd = open_dir(self.archive, private=True)
        try:
            manifest, bodies = r.validate_archive(fd, '2026-07-01')
        finally:
            os.close(fd)
        self.assertEqual(bodies, {'2026-07-01/'+name: raw for name, raw in self.original.items()})
        archive_before = {p.name: p.read_bytes() for p in self.archive.iterdir()}
        self.rotate(archive=True)
        self.assertEqual(archive_before, {p.name: p.read_bytes() for p in self.archive.iterdir()})
        report = self.rotate(archive=True, prune=True)
        self.assertEqual(len(report['results'][0]['removed']), 2)
        self.assertEqual(len(self.rotate(archive=True, prune=True)['results'][0]['already_absent']), 2)
        for path, inode in self.inodes.items():
            self.assertEqual(path.stat().st_ino, inode)
        self.assertEqual((self.root / '2026-10-09/today.md').read_bytes(), b'keep')
        self.assertEqual((self.root / '2026-10-10/today.md').read_bytes(), b'keep')
        self.assertEqual(manifest['schema_version'], 1)
        self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in self.archive.iterdir()))

    def test_corrupt_archive_never_prunes(self):
        self.rotate(archive=True)
        tar = self.archive / '2026-07-01.tar'
        tar.write_bytes(b'corrupt')
        with self.assertRaises(ValueError):
            self.rotate(archive=True, prune=True)
        self.assertTrue((self.old / 'a.md').exists())

    def test_partial_publication_keeps_sources_and_refuses_orphan_overwrite(self):
        real_write = r.write_at
        def fail_manifest(fd, name, raw):
            if name.endswith('.json'):
                raise OSError('crash before manifest')
            real_write(fd, name, raw)
        with patch.object(r, 'write_at', side_effect=fail_manifest):
            with self.assertRaises(OSError):
                self.rotate(archive=True)
        self.assertTrue((self.old / 'a.md').exists())
        with self.assertRaises(ValueError):
            self.rotate(archive=True, prune=True)

    def test_partial_prune_resume_and_unlisted_new_file_kept(self):
        self.rotate(archive=True)
        (self.old / 'new.md').write_bytes(b'new after snapshot')
        def crash(_):
            raise RuntimeError('after first durable unlink')
        with self.assertRaises(RuntimeError):
            self.rotate(archive=True, prune=True, after_unlink=crash)
        self.assertEqual(sum((self.old / name).exists() for name in self.original), 1)
        report = self.rotate(archive=True, prune=True)
        self.assertEqual(len(report['results'][0]['removed']), 1)
        self.assertEqual(len(report['results'][0]['already_absent']), 1)
        self.assertEqual((self.old / 'new.md').read_bytes(), b'new after snapshot')

    def test_changed_source_blocks_entire_prune(self):
        self.rotate(archive=True)
        (self.old / 'b.md').write_bytes(b'changed')
        with self.assertRaises(ValueError):
            self.rotate(archive=True, prune=True)
        self.assertTrue((self.old / 'a.md').exists())

    def test_symlink_hardlink_writable_and_lock_refusal(self):
        (self.old / 'a.md').unlink()
        target = self.runtime / 'external'
        target.write_bytes(b'outside')
        (self.old / 'a.md').symlink_to(target)
        with self.assertRaises(OSError):
            self.rotate(archive=True)
        (self.old / 'a.md').unlink()
        os.link(target, self.old / 'a.md')
        with self.assertRaises(ValueError):
            self.rotate(archive=True)
        (self.old / 'a.md').unlink()
        (self.old / 'a.md').write_bytes(b'regular')
        (self.old / 'a.md').chmod(0o666)
        with self.assertRaises(ValueError):
            self.rotate(archive=True)
        (self.old / 'a.md').chmod(0o600)
        self.lock.unlink()
        with self.assertRaises(FileNotFoundError):
            self.rotate(archive=True)
        self.assertFalse(self.lock.exists())

    def test_writer_lock_contention_and_root_swap(self):
        with self.lock.open('r+') as held:
            fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(TimeoutError):
                self.rotate(archive=True, lock_wait=.02)
        original = r.read_at
        swapped = False
        def swap(fd, name, *args, **kwargs):
            nonlocal swapped
            result = original(fd, name, *args, **kwargs)
            if name.endswith('.md') and not swapped:
                swapped = True
                self.root.rename(self.runtime / 'moved')
                self.root.mkdir()
            return result
        with patch.object(r, 'read_at', side_effect=swap):
            with self.assertRaises(ValueError):
                self.rotate(archive=True)
        self.assertTrue((self.runtime / 'moved/2026-07-01/a.md').exists())

    def test_bounds_and_archive_root_inside_source(self):
        with self.assertRaises(ValueError):
            self.rotate(archive=True, max_files=1)
        with self.assertRaises(ValueError):
            r.rotate(self.runtime, self.root / 'archive')
        with self.assertRaises(ValueError):
            self.rotate(prune=True)


if __name__ == '__main__':
    unittest.main()
