import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from verify_executable_input_manifest import COMPILED_PUBLIC_INPUTS, MANIFEST_V1, verify


class VerifyExecutableInputManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "root"
        (self.root / "src").mkdir(parents=True)
        (self.root / "config").mkdir()
        self.input = self.root / "src" / "example.rs"
        self.input.write_bytes(b"fn main() {}\n")
        self.manifest = Path(self.temp.name) / "manifest.json"
        self.manifest.write_text(
            json.dumps(
                [
                    {
                        "path": "src/example.rs",
                        "length": self.input.stat().st_size,
                        "sha256": hashlib.sha256(self.input.read_bytes()).hexdigest(),
                    }
                ]
            ),
            encoding="utf-8",
        )

    def test_exact_inputs_and_explicit_activation_extra(self):
        activation = self.root / "config" / "selection" / "selection_activation.v1.json"
        activation.parent.mkdir()
        activation.write_text("{}", encoding="utf-8")
        count, errors = verify(
            self.manifest,
            self.root,
            {"config/selection/selection_activation.v1.json"},
            activation_ready=True,
        )
        self.assertEqual(count, 1)
        self.assertEqual(errors, [])

    def test_modified_bytes_and_unexpected_extra_fail(self):
        self.input.write_bytes(b"fn main() { panic!() }\n")
        (self.root / "src" / "extra.rs").write_text("", encoding="utf-8")
        _, errors = verify(self.manifest, self.root, set())
        self.assertTrue(any("input differs: src/example.rs" == error for error in errors))
        self.assertTrue(any("unexpected input file: src/extra.rs" == error for error in errors))

    def test_activation_ready_rejects_allowlisted_src_config_files(self):
        (self.root / "src" / ".DS_Store").write_bytes(b"finder")
        (self.root / "config" / ".DS_Store").write_bytes(b"finder")
        activation = self.root / "config" / "selection" / "selection_activation.v1.json"
        activation.parent.mkdir()
        activation.write_text("{}", encoding="utf-8")
        command = [
            sys.executable,
            str(Path(__file__).with_name("verify_executable_input_manifest.py")),
            str(self.manifest),
            str(self.root),
            "--allow-extra", "src/.DS_Store",
            "--allow-extra", "config/.DS_Store",
            "--allow-extra", "config/selection/selection_activation.v1.json",
        ]
        generic = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(generic.returncode, 0, generic.stdout + generic.stderr)

        activation_ready = subprocess.run(
            command + ["--activation-ready"], capture_output=True, text=True, check=False
        )
        self.assertEqual(activation_ready.returncode, 1)
        self.assertIn("activation input cannot be allowed extra: src/.DS_Store", activation_ready.stdout)
        self.assertIn("activation input cannot be allowed extra: config/.DS_Store", activation_ready.stdout)
        self.assertNotIn("selection_activation.v1.json", activation_ready.stdout)

    def test_symlink_and_parent_path_fail(self):
        self.input.unlink()
        self.input.symlink_to(Path(self.temp.name) / "elsewhere")
        _, errors = verify(self.manifest, self.root, set())
        self.assertTrue(any("symlink" in error for error in errors))
        rows = json.loads(self.manifest.read_text(encoding="utf-8"))
        rows[0]["path"] = "src/../outside.rs"
        self.manifest.write_text(json.dumps(rows), encoding="utf-8")
        _, errors = verify(self.manifest, self.root, set())
        self.assertTrue(any("invalid or duplicate" in error for error in errors))

    def test_v2_requires_public_contracts_and_detects_changed_metadata(self):
        rows = json.loads(self.manifest.read_text(encoding="utf-8"))
        for name in sorted(COMPILED_PUBLIC_INPUTS):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"TEST_CODE_public_contract")
            rows.append({
                "path": name, "length": path.stat().st_size,
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            })
        self.manifest.write_text(json.dumps(rows), encoding="utf-8")
        count, errors = verify(self.manifest, self.root, set(), activation_ready=True)
        self.assertEqual(count, 1 + len(COMPILED_PUBLIC_INPUTS))
        self.assertEqual(errors, [])

        metadata = "contracts/external_v1_current/bundle-metadata.json"
        (self.root / metadata).write_bytes(b"TEST_CODE_changed_contract")
        _, errors = verify(self.manifest, self.root, set(), activation_ready=True)
        self.assertIn(f"input differs: {metadata}", errors)
        self.manifest.write_text(
            json.dumps([row for row in rows if row["path"] != metadata]), encoding="utf-8"
        )
        _, errors = verify(self.manifest, self.root, set(), activation_ready=True)
        self.assertIn(f"compiled public input missing from manifest: {metadata}", errors)

    def test_current_public_contracts_cannot_use_v1_activation_scope(self):
        (self.root / "contracts/external_v1_current").mkdir(parents=True)
        _, errors = verify(
            self.manifest, self.root, set(), activation_ready=True,
            input_manifest_version=MANIFEST_V1,
        )
        self.assertIn(
            "current public contracts require executable input manifest v2", errors
        )

    def test_missing_current_directory_cannot_downgrade_sealed_build(self):
        build = self.root / "build.rs"
        build.write_bytes(b'let source = "contracts/external_v1_current/market.proto";\n')
        rows = json.loads(self.manifest.read_text(encoding="utf-8"))
        rows.append({
            "path": "build.rs", "length": build.stat().st_size,
            "sha256": hashlib.sha256(build.read_bytes()).hexdigest(),
        })
        self.manifest.write_text(json.dumps(rows), encoding="utf-8")
        _, errors = verify(self.manifest, self.root, set(), activation_ready=True)
        self.assertIn(
            "compiled public input missing from manifest: contracts/external_v1_current/market.proto",
            errors,
        )
        _, errors = verify(
            self.manifest, self.root, set(), activation_ready=True,
            input_manifest_version=MANIFEST_V1,
        )
        self.assertIn("current public contracts require executable input manifest v2", errors)

    def test_dangling_current_directory_cannot_downgrade_scope(self):
        (self.root / "contracts").mkdir()
        (self.root / "contracts/external_v1_current").symlink_to(
            Path(self.temp.name) / "missing-public-inputs", target_is_directory=True,
        )
        _, errors = verify(self.manifest, self.root, set(), activation_ready=True)
        self.assertIn(
            "compiled public input missing from manifest: contracts/external_v1_current/market.proto",
            errors,
        )

    def test_declared_current_inputs_require_v2_even_when_all_contracts_missing(self):
        rows = json.loads(self.manifest.read_text(encoding="utf-8"))
        rows.append({
            "path": "contracts/external_v1_current/market.proto", "length": 1,
            "sha256": hashlib.sha256(b"x").hexdigest(),
        })
        self.manifest.write_text(json.dumps(rows), encoding="utf-8")
        _, errors = verify(self.manifest, self.root, set(), activation_ready=True)
        self.assertIn(
            "compiled public input missing from manifest: contracts/external_v1_current/bundle-metadata.json",
            errors,
        )


if __name__ == "__main__":
    unittest.main()
