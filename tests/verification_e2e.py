"""Exercise verification entrypoints with isolated failing external commands."""

from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import zipfile
import hashlib
import json

ROOT = Path(__file__).resolve().parent.parent


def executable(path, content):
    path.write_text(content)
    path.chmod(0o755)


def run():
    with tempfile.TemporaryDirectory(prefix="lankide-verification-") as directory:
        root = Path(directory)
        scripts = root / "scripts"
        scripts.mkdir()
        shutil.copy2(ROOT / "scripts/verify-all", scripts / "verify-all")
        shutil.copy2(ROOT / "scripts/run-hermetic-tests", scripts / "run-hermetic-tests")
        (root / "tests").mkdir()
        (root / "tests/e2e-modules.tsv").write_text(
            "# module_id\tpackage\tkind\tclassification\tcommand\tscenario\tprerequisites\tunattended\tstatus\n"
            "pending\tproduct\tcrate\thermetic\tfalse\tUnfinished business scenario\tNone\tyes\tpending\n"
        )
        env = {**os.environ, "PATH": f"{root}:{os.environ['PATH']}"}
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "hermetic"], env=env, capture_output=True, text=True)
        assert result.returncode == 3 and "PENDING" in result.stderr, result.stdout + result.stderr
        (root / "tests/e2e-modules.tsv").write_text(
            (root / "tests/e2e-modules.tsv").read_text().replace("\tpending\n", "\timplemented\n")
        )
        executable(root / "cargo", '#!/bin/sh\nexit 42\n')
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "hermetic"], env=env, capture_output=True, text=True)
        assert result.returncode == 42, result.stdout + result.stderr

        # Run every Cargo integration executable, keep bin artifacts out, and
        # report a scenario failure only after the other suites finish cleanup.
        calls = root / "calls"
        calls.mkdir()
        artifacts = []
        for name, kind, code in [("passing", "test", 0), ("failing", "test", 7), ("product", "bin", 9)]:
            scenario = root / name
            executable(scenario, f"#!/bin/sh\ntouch '{calls / name}'\nexit {code}\n")
            artifacts.append({"reason": "compiler-artifact", "package_id": name,
                              "executable": str(scenario),
                              "target": {"kind": [kind], "src_path": str(root / "tests/e2e/main.rs")}})
        executable(root / "cargo", "#!/usr/bin/env python3\nimport sys\n"
                   f"artifacts = {artifacts!r}\n"
                   "if '--message-format=json' in sys.argv:\n"
                   " import json\n"
                   " for artifact in artifacts: print(json.dumps(artifact))\n")
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "hermetic"], env=env, capture_output=True, text=True)
        assert result.returncode == 1, result.stdout + result.stderr
        assert {path.name for path in calls.iterdir()} == {"passing", "failing"}
        executable(root / "failing", "#!/bin/sh\nexit 0\n")
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "hermetic"], env=env, capture_output=True, text=True)
        assert result.returncode == 0, result.stdout + result.stderr
        executable(root / "cargo", "#!/bin/sh\ncase \"$*\" in *--message-format=json*) exit 43;; esac\nexit 0\n")
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "hermetic"], env=env, capture_output=True, text=True)
        assert result.returncode == 43, result.stdout + result.stderr

        # Release process checks must propagate failures at the public gate.
        desktop_tests = root / "apps/desktop/tests"
        desktop_tests.mkdir(parents=True)
        bootstrap = desktop_tests / "bootstrap_e2e.py"
        bootstrap.write_text("raise SystemExit(44)\n")
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "release-regressions"], env=env, capture_output=True, text=True)
        assert result.returncode == 44, result.stdout + result.stderr
        bootstrap.write_text("raise SystemExit(0)\n")
        (root / "tests/verification_e2e.py").write_text("raise SystemExit(45)\n")
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "release-regressions"], env=env, capture_output=True, text=True)
        assert result.returncode == 45, result.stdout + result.stderr

        # Composition is a required subgate, with live opt-in and exit propagation.
        composition_env = {**env, "LANKIDE_E2E_RUN_ID": "composition-negative", "LANKIDE_E2E_LOCK_ROOT": str(root / "locks")}
        composition_env.pop("LANKIDE_E2E_LIVE", None)
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "live-composition"], env=composition_env, capture_output=True, text=True)
        assert result.returncode == 4, result.stdout + result.stderr
        executable(scripts / "verify-live-composition", "#!/bin/sh\nexit 43\n")
        composition_env["LANKIDE_E2E_LIVE"] = "1"
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "live-composition"], env=composition_env, capture_output=True, text=True)
        assert result.returncode == 43, result.stdout + result.stderr
        resume_dir = root / "target/e2e-artifacts/full-release/composition-negative"
        resume_dir.mkdir(parents=True)
        steps = [{"id": name, "status": "passed", "cleanup": "passed"} for name in ("release-build", "hermetic", "live-provider", "live-harness")]
        steps.append({"id": "desktop-visual", "status": "failed"})
        (resume_dir / "index.json").write_text(json.dumps({"run_id": "composition-negative", "started_at": "test", "steps": steps}))
        (resume_dir / "result.json").write_text(json.dumps({"status": "failed"}))
        result = subprocess.run([str(scripts / "verify-all"), "--gate", "full-release", "--resume"], env=composition_env, capture_output=True, text=True)
        assert result.returncode != 0 and "live-composition lacks one clean passed result" in result.stderr, result.stdout + result.stderr

        shutil.copy2(ROOT / "scripts/verify-production-boundary", scripts / "verify-production-boundary")
        package = root / "apps/product"
        (package / "src").mkdir(parents=True)
        manifest = package / "Cargo.toml"
        manifest.write_text('[package]\nname = "product"\nversion = "0.1.0"\n')
        source = package / "src/main.rs"
        source.write_text("fn main() {}\n")
        command = [str(scripts / "verify-production-boundary")]
        assert subprocess.run(command, capture_output=True).returncode == 0
        for symbol in ("MockManagementClient", "QueueEventSource", "AtomicFailure", "deepseek-v4-flash", "gpt-5.6-luna"):
            source.write_text(f"fn main() {{ {symbol}::sample(); }}\n")
            assert subprocess.run(command, capture_output=True).returncode != 0, symbol
        source.write_text('const DATA: &str = include_str!("../tests/data.json");\n')
        assert subprocess.run(command, capture_output=True).returncode != 0
        test_assets = root / "apps/desktop/tests/assets"
        test_assets.mkdir(parents=True)
        (test_assets / "portrait.png").write_bytes(b"isolated test image")
        production_assets = package / "assets"
        production_assets.mkdir()
        copied = production_assets / "renamed.png"
        copied.write_bytes(b"isolated test image")
        source.write_text("fn main() {}\n")
        assert subprocess.run(command, capture_output=True).returncode != 0, "renamed test image escaped boundary"
        copied.unlink()
        manifest.write_text(manifest.read_text() + '[dependencies]\nfixtures = { package = "lankide-testkit", path = "../../testkit" }\n')
        assert subprocess.run(command, capture_output=True).returncode != 0
        manifest.write_text(manifest.read_text().replace('[dependencies]', '[dev-dependencies]'))
        assert subprocess.run(command, capture_output=True).returncode == 0
        # A release resume must reject changed bytes even if timestamps are preserved.
        shutil.copy2(ROOT / "scripts/release-provenance", scripts / "release-provenance")
        (root / "Cargo.toml").write_text('[workspace]\nmembers = []\n')
        (root / "Cargo.lock").write_text('version = 4\n')
        for name in ("LICENSE", "THIRD_PARTY_LICENSES.md"):
            shutil.copy2(ROOT / name, root / name)
        binary_dir = root / "release"
        binary_dir.mkdir()
        for name in ("lankide", "lankide-desktop", "lankide-runtime-daemon", "lankide-computer-daemon"):
            executable(binary_dir / name, "#!/bin/sh\nexit 0\n")
        snapshot, provenance = root / "snapshot.json", root / "provenance.json"
        provenance_tool = str(scripts / "release-provenance")
        def invoke(*arguments):
            return subprocess.run([provenance_tool, *map(str, arguments)], capture_output=True, text=True)
        assert invoke("snapshot", "--output", snapshot).returncode == 0
        assert invoke("record", "--output", provenance, "--binary-dir", binary_dir, "--source-snapshot", snapshot).returncode == 0
        verify = ("verify", "--output", provenance, "--binary-dir", binary_dir)
        assert invoke(*verify).returncode == 0
        for changed in (source, binary_dir / "lankide", scripts / "verify-all", root / "LICENSE"):
            original, stat = changed.read_bytes(), changed.stat()
            changed.write_bytes(original + b"\n# changed bytes\n")
            os.utime(changed, ns=(stat.st_atime_ns, stat.st_mtime_ns))
            assert invoke(*verify).returncode != 0, f"timestamp-preserving edit escaped: {changed}"
            changed.write_bytes(original)
            os.utime(changed, ns=(stat.st_atime_ns, stat.st_mtime_ns))
        assert invoke(*verify).returncode == 0
        visual_tests = root / "apps/desktop/tests"
        visual_tests.mkdir(parents=True, exist_ok=True)
        visual_gate = visual_tests / "window_acceptance.py"
        shutil.copy2(ROOT / "apps/desktop/tests/window_acceptance.py", visual_gate)
        review_dir = root / "window-review"
        visual_env = {**os.environ, "LANKIDE_VISUAL_BINARY": str(binary_dir / "lankide-desktop"), "LANKIDE_RELEASE_BIN_DIR": str(binary_dir)}
        visual_env.pop("LANKIDE_VISUAL_SCENARIOS", None)
        visual_env.pop("LANKIDE_VISUAL_REVIEW", None)
        def visual(*arguments, env=visual_env):
            return subprocess.run(["python3", str(visual_gate), *map(str, arguments)], env=env, capture_output=True, text=True)
        assert visual().returncode == 4, "missing real-window evidence must not pass"
        assert visual("--prepare", review_dir).returncode == 0
        assert visual("--review", review_dir / "review.json").returncode != 0, "pending template must not pass"
        assert visual("--review", review_dir / "review.json", env={**visual_env, "LANKIDE_VISUAL_SCENARIOS": "agent-list"}).returncode != 0, "partial visual selection must not pass"
        # Exercise image formats through the public evidence gate. These
        # deliberately undersized test images must never satisfy acceptance.
        review_path = review_dir / "review.json"
        review = json.loads(review_path.read_text())
        review.update(status="passed", reviewer="isolated validator regression")
        for item in review["scenarios"]:
            item.update(status="passed", observations="test-only validator input")
        for name in ("workspace-cover.jpg", "workspace-cover.png"):
            data = (ROOT / "apps/desktop/tests/assets" / name).read_bytes()
            (review_dir / name).write_bytes(data)
            review["scenarios"][0]["screenshot"] = {"path": name, "sha256": hashlib.sha256(data).hexdigest()}
            review_path.write_text(json.dumps(review))
            result = visual("--review", review_path)
            assert result.returncode != 0 and "complete product window" in result.stderr, result.stderr
        malformed = b"\xff\xd8\xff\xc0\x00\xff\xff\xd9"
        (review_dir / "broken.jpg").write_bytes(malformed)
        review["scenarios"][0]["screenshot"] = {"path": "broken.jpg", "sha256": hashlib.sha256(malformed).hexdigest()}
        review_path.write_text(json.dumps(review))
        result = visual("--review", review_path)
        assert result.returncode != 0 and "invalid PNG or JPEG" in result.stderr, result.stderr
        # Package acceptance must reject a stale binary, modified signed copy,
        # and a different archive even when its ZIP CRCs are internally valid.
        shutil.copy2(ROOT / "scripts/package-provenance", scripts / "package-provenance")
        bundle = root / "packaged/Lankide.app"
        layout = {
            "lankide-desktop": "Contents/MacOS/Lankide",
            "lankide-runtime-daemon": "Contents/Library/LaunchServices/lankide-runtime-daemon",
            "lankide-computer-daemon": "Contents/Library/LaunchServices/lankide-computer-daemon",
            "lankide": "Contents/Resources/bin/lankide",
        }
        for name, relative in layout.items():
            destination = bundle / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(binary_dir / name, destination)
        (bundle / "Contents/Info.plist").write_text("test-only bundle metadata")
        (bundle / "Contents/Resources/AppIcon.icns").write_bytes(b"test-only icon")
        for name in ("LICENSE", "THIRD_PARTY_LICENSES.md"):
            shutil.copy2(root / name, bundle / "Contents/Resources" / name)
        launch_agents = bundle / "Contents/Library/LaunchAgents"
        launch_agents.mkdir(parents=True)
        for name in ("com.lankide.runtime.plist", "com.lankide.computer.plist"):
            (launch_agents / name).write_text("test-only launch agent metadata")
        assert invoke("snapshot", "--output", snapshot).returncode == 0
        assert invoke("record", "--output", provenance, "--binary-dir", binary_dir, "--source-snapshot", snapshot).returncode == 0
        stage, package_receipt = root / "stage.json", root / "package.json"
        archive = root / "package.zip"
        def make_archive(changed=False):
            with zipfile.ZipFile(archive, "w") as zipped:
                for path in bundle.rglob("*"):
                    if path.is_file():
                        info = zipfile.ZipInfo.from_file(path, (Path(bundle.name) / path.relative_to(bundle)).as_posix())
                        data = path.read_bytes()
                        if changed and path.name == "Info.plist":
                            data += b"stale archive payload"
                        zipped.writestr(info, data)
        def package(action, *arguments):
            return subprocess.run([str(scripts / "package-provenance"), action,
                "--binary-dir", str(binary_dir), "--build-provenance", str(provenance),
                "--bundle", str(bundle), *map(str, arguments)], capture_output=True, text=True)
        assert package("stage", "--output", stage).returncode == 0
        make_archive()
        record = ("--stage", stage, "--archive", archive, "--output", package_receipt)
        assert package("record", *record).returncode == 0
        check_package = ("--archive", archive, "--output", package_receipt)
        assert package("verify", *check_package).returncode == 0
        for path in (binary_dir / "lankide", bundle / "Contents/MacOS/Lankide"):
            original = path.read_bytes()
            path.write_bytes(original + b"changed")
            assert package("verify", *check_package).returncode != 0, "changed executable accepted"
            if path.is_relative_to(bundle):
                assert package("stage", "--output", stage).returncode != 0, "different staged binary accepted"
            path.write_bytes(original)
        make_archive(changed=True)
        assert package("verify", *check_package).returncode != 0, "different valid ZIP accepted"
        assert package("record", *record).returncode != 0, "archive mismatch accepted at packaging"
    print("verification entrypoint E2E: PASS (failure propagation, production isolation, release provenance, actual-window evidence required)")


if __name__ == "__main__":
    run()
