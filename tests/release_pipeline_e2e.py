"""Release scenarios with isolated artifacts and external Apple/GitHub doubles."""
import json
import os
from pathlib import Path
import plistlib
import runpy
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
ASSETS = runpy.run_path(str(ROOT / "scripts/release-assets"))


def executable(path, text):
    path.write_text(text)
    path.chmod(0o755)


class ReleaseScenarios(unittest.TestCase):
    def test_complete_matrix_is_required_and_bound_to_verified_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "inputs"
            source.mkdir()
            version = "0.1.0"
            for name in ASSETS["expected_assets"](version):
                (source / name).write_text(name)
            (source / "install.sh").write_text('VERSION="0.1.0"\n')
            (source / "sbom.spdx.json").write_text(json.dumps({"spdxVersion": "SPDX-2.3"}))
            for path in source.glob("*.sha256"):
                base = path.name.removesuffix(".sha256")
                names = [base] if base.endswith(".tar.gz") else [base + ".zip", base + ".dmg"]
                path.write_text("".join(f"{ASSETS['digest'](source / name)}  {name}\n" for name in names))
            for path in source.glob("*.verification.json"):
                base = path.name.removesuffix(".verification.json")
                archive = base if base.endswith(".tar.gz") else base + ".zip"
                result = {"status": "passed", "version": version, "archive_sha256": ASSETS["digest"](source / archive),
                    "distribution_signing": "developer-id-notarized" if "-macos-" in path.name else "none"}
                if not base.endswith(".tar.gz"):
                    result.update(cleanup="passed", dmg_sha256=ASSETS["digest"](source / (base + ".dmg")))
                path.write_text(json.dumps(result))
            for path in source.glob("*.package.json"):
                base = path.name.removesuffix(".package.json")
                path.write_text(json.dumps({"archive_sha256": ASSETS["digest"](source / (base + ".zip")),
                    "build_sha256": ASSETS["digest"](source / (base + ".build.json"))}))
            assemble = lambda destination: ASSETS["assemble"](source, root / destination, version, "a" * 40)
            assemble("complete")
            evidence = source / "meerkat-machine-0.1.0-linux-arm64.tar.gz.verification.json"
            original = evidence.read_bytes()
            evidence.unlink()
            with self.assertRaisesRegex(ValueError, "missing release"):
                assemble("missing")
            evidence.write_bytes(original)
            evidence = source / "Meerkat-0.1.0-macos-arm64.verification.json"
            original = evidence.read_bytes()
            result = json.loads(original)
            result["distribution_signing"] = "development"
            evidence.write_text(json.dumps(result))
            with self.assertRaisesRegex(ValueError, "formal macOS"):
                assemble("development-signature")
            evidence.write_bytes(original)
            archive = source / "Meerkat-0.1.0-macos-arm64.zip"
            archive.write_bytes(archive.read_bytes() + b"changed")
            with self.assertRaises(ValueError):
                assemble("tampered")
            self.assertFalse((root / "tampered").exists())

    def test_rejected_notarization_never_staples_or_reports_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            calls = root / "calls"
            executable(root / "xcrun", '''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
with open(os.environ['CALLS'], 'a') as log: log.write(' '.join(sys.argv[1:]) + '\\n')
if sys.argv[1:3] == ['notarytool', 'submit']:
    print(json.dumps({'id': 'submission', 'status': os.environ['NOTARY_STATUS']}))
elif sys.argv[1:3] == ['notarytool', 'log']:
    Path(sys.argv[-1]).write_text('{}')
''')
            artifact = root / "release.dmg"
            artifact.write_bytes(b"isolated disk image")
            env = {**os.environ, "PATH": f"{root}:{os.environ['PATH']}", "CALLS": str(calls),
                "MEERKAT_NOTARY_KEY": str(root / "key"), "MEERKAT_NOTARY_KEY_ID": "test-id",
                "MEERKAT_NOTARY_ISSUER": "test-issuer", "MEERKAT_NOTARY_LOG_DIR": str(root / "logs")}
            for status in ("Invalid", "In Progress", "Accepted"):
                calls.write_text("")
                result = subprocess.run([str(ROOT / "scripts/notarize-macos"), str(artifact)],
                    env={**env, "NOTARY_STATUS": status}, capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, status == "Accepted", result.stderr)
                self.assertEqual("stapler staple" in calls.read_text(), status == "Accepted")
                self.assertTrue((root / "logs/release.dmg.log.json").is_file())

    def test_public_release_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "publish"
            directory.mkdir()
            (directory / "release-manifest.json").write_text(json.dumps({"version": "0.1.0", "commit": "a" * 40, "assets": {}}))
            (directory / "SHA256SUMS").write_text("")
            calls = root / "calls"
            executable(root / "gh", '''#!/usr/bin/env python3
import json, os, sys
with open(os.environ['CALLS'], 'a') as log: log.write(' '.join(sys.argv[1:]) + '\\n')
if sys.argv[1:3] == ['release', 'view']: print(json.dumps({'isDraft': False, 'assets': []}))
''')
            result = subprocess.run([str(ROOT / "scripts/publish-release"), str(directory)], capture_output=True, text=True,
                env={**os.environ, "PATH": f"{root}:{os.environ['PATH']}", "CALLS": str(calls),
                     "RELEASE_TAG": "v0.1.0", "RELEASE_COMMIT": "a" * 40,
                     "GITHUB_REPOSITORY": "example/product", "RELEASE_PRERELEASE": "false"})
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("release upload", calls.read_text())
            self.assertNotIn("release edit", calls.read_text())

    def test_tag_checkout_and_workflow_identity_must_match(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "scripts").mkdir()
            (root / "apps/desktop").mkdir(parents=True)
            shutil.copy2(ROOT / "scripts/release-preflight", root / "scripts/release-preflight")
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n')
            (root / "apps/desktop/Cargo.toml").write_text('[package]\nversion = "0.1.0"\n')
            (root / "apps/desktop/tauri.conf.json").write_text(json.dumps({"version": "0.1.0"}))
            (root / "apps/desktop/Info.plist").write_bytes(plistlib.dumps({
                "CFBundleVersion": "0.1.0", "CFBundleShortVersionString": "0.1.0"}))
            def git(*arguments):
                return subprocess.check_output(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", *arguments], cwd=root, text=True).strip()
            git("init", "-q", "-b", "main")
            git("add", ".")
            git("-c", "user.name=Release scenario", "-c", "user.email=release@example.invalid", "commit", "-qm", "release fixture")
            commit = git("rev-parse", "HEAD")
            git("tag", "v0.1.0")
            git("update-ref", "refs/remotes/origin/main", commit)
            env = {**os.environ, "RELEASE_TAG": "v0.1.0", "DEFAULT_BRANCH": "main", "EVENT_NAME": "workflow_dispatch",
                "DISPATCH_REF": "refs/tags/v0.1.0", "WORKFLOW_COMMIT": commit, "GITHUB_OUTPUT": str(root / "output")}
            command = [str(root / "scripts/release-preflight")]
            self.assertEqual(subprocess.run(command, env=env, capture_output=True).returncode, 0)
            self.assertNotEqual(subprocess.run(command, env={**env, "WORKFLOW_COMMIT": "f" * 40}, capture_output=True).returncode, 0)
            self.assertNotEqual(subprocess.run(command, env={**env, "DISPATCH_REF": "refs/heads/unreviewed"}, capture_output=True).returncode, 0)
            path = root / "apps/desktop/tauri.conf.json"
            path.write_text(path.read_text().replace('"0.1.0"', '"0.2.0"'))
            self.assertNotEqual(subprocess.run(command, env=env, capture_output=True).returncode, 0)


if __name__ == "__main__":
    unittest.main()
