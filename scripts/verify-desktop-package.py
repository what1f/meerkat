#!/usr/bin/env python3
"""Verify an extracted Linux or Windows Desktop package on its native runner."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import time
import tomllib
import urllib.parse
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare_bootstrap_home(home: Path) -> None:
    home.mkdir(parents=True, mode=0o700)
    (home / "runtime.json").write_text(
        json.dumps({"machine_link_listen": "127.0.0.1:0", "remote_access_port": 0}),
        encoding="utf-8",
    )


def wait_for_smoke(process: subprocess.Popen, home: Path, timeout: float = 60.0):
    deadline = time.monotonic() + timeout
    discovered_url = None
    url_file = home / "run" / "management.url"
    while time.monotonic() < deadline:
        if discovered_url is None and url_file.is_file():
            text = url_file.read_text(encoding="utf-8").strip()
            if text:
                discovered_url = text
        code = process.poll()
        if code is not None:
            return code, discovered_url
        time.sleep(0.1)
    process.kill()
    process.wait()
    raise RuntimeError("Desktop bootstrap smoke timed out")


def origin(url: str) -> str:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "http" or parsed.hostname != "127.0.0.1" or not parsed.port:
        raise ValueError(f"unexpected loopback URL: {url}")
    return f"{parsed.scheme}://{parsed.hostname}:{parsed.port}"


def assert_navigation(observed: dict, discovered_url: str | None) -> None:
    url = observed.get("navigation_url")
    if not isinstance(url, str) or not url:
        raise ValueError("missing navigation_url")
    if not discovered_url:
        raise ValueError("management.url was never written during bootstrap smoke")
    if origin(url) != origin(discovered_url):
        raise ValueError(f"navigation_url {url} does not match discovered runtime {discovered_url}")


def run_bootstrap_smoke(desktop_exe: Path, home: Path, artifacts: Path) -> None:
    prepare_bootstrap_home(home)
    bootstrap_result = artifacts / "bootstrap.json"
    env = {key: value for key, value in os.environ.items() if not key.startswith(("MEERKAT_", "FX_"))}
    env["MEERKAT_HOME"] = str(home)
    with (artifacts / "desktop.log").open("w", encoding="utf-8") as log:
        process = subprocess.Popen(
            [str(desktop_exe), "--local-bootstrap-smoke", "--result", str(bootstrap_result)],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        code, discovered_url = wait_for_smoke(process, home, timeout=60.0)
    if (home / "logs").exists():
        shutil.copytree(home / "logs", artifacts / "services", dirs_exist_ok=True)
    if code != 0:
        raise ValueError(f"Desktop bootstrap failed (exit {code}); see {artifacts / 'desktop.log'}")
    observed = json.loads(bootstrap_result.read_text(encoding="utf-8"))
    if observed.get("status") != "passed" or not all(
        observed.get(key) is True for key in ("runtime_ready", "machine_connected", "management")
    ):
        raise ValueError(f"Desktop did not pass all startup checks: {observed}")
    assert_navigation(observed, discovered_url)
    for leftover in ("runtime.lock", "machine.lock"):
        if (home / "run" / leftover).exists():
            raise ValueError(f"Desktop left lock file behind: {leftover}")


def verify_windows_installer(installer: Path, artifacts: Path, has_builtin_agent: bool) -> None:
    with tempfile.TemporaryDirectory(prefix="meerkat-win-inst-") as temporary:
        root = Path(temporary)
        install_dir = root / "install"
        install_dir.mkdir()
        sentinel = install_dir / "user-notes.txt"
        sentinel.write_text("keep-my-file\n", encoding="utf-8")
        nested_dir = install_dir / "licenses"
        nested_dir.mkdir()
        nested_sentinel = nested_dir / "user-license.txt"
        nested_sentinel.write_text("keep-my-nested-file\n", encoding="utf-8")
        optional_sentinel = install_dir / "meerkat-agent.exe"
        if not has_builtin_agent:
            optional_sentinel.write_text("keep-user-optional-binary\n", encoding="utf-8")

        subprocess.run([str(installer), "/S", f"/D={install_dir}"], check=True, timeout=120)
        desktop_exe = install_dir / "meerkat-desktop.exe"
        if not desktop_exe.is_file():
            raise ValueError("Windows installer did not install meerkat-desktop.exe")
        run_bootstrap_smoke(desktop_exe, root / "home", artifacts / "installer-smoke")

        # Re-run installer over the same directory to verify in-place upgrade path.
        subprocess.run([str(installer), "/S", f"/D={install_dir}"], check=True, timeout=120)

        uninstaller = install_dir / "Uninstall.exe"
        if not uninstaller.is_file():
            raise ValueError("Windows installer did not create Uninstall.exe")
        subprocess.run([str(uninstaller), "/S", f"_?={install_dir}"], check=True, timeout=120)
        if not sentinel.is_file() or sentinel.read_text(encoding="utf-8") != "keep-my-file\n":
            raise ValueError("Windows uninstaller deleted pre-existing user file in install directory")
        if not nested_sentinel.is_file() or nested_sentinel.read_text(encoding="utf-8") != "keep-my-nested-file\n":
            raise ValueError("Windows uninstaller deleted pre-existing nested file in licenses directory")
        if not has_builtin_agent and (
            not optional_sentinel.is_file()
            or optional_sentinel.read_text(encoding="utf-8") != "keep-user-optional-binary\n"
        ):
            raise ValueError("Windows uninstaller deleted unbundled meerkat-agent.exe")
        for removed in ("meerkat-desktop.exe", "meerkatd.exe", "meerkat-machine.exe", "meerkat.exe"):
            if (install_dir / removed).exists():
                raise ValueError(f"Windows uninstaller left product file behind: {removed}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--build-provenance", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--verify-installer", action="store_true")
    args = parser.parse_args()

    archive = args.archive.resolve()
    version = tomllib.loads((ROOT / "apps/desktop/Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    build_hash = sha256(args.build_provenance)
    build = json.loads(args.build_provenance.read_text(encoding="utf-8"))
    archive_hash = sha256(archive)

    is_windows = archive.suffix == ".zip"
    if is_windows:
        base = archive.name.removesuffix(".zip")
        installer = archive.with_name(f"{base}-setup.exe")
        if not installer.is_file():
            raise ValueError(f"Windows installer is missing: {installer}")
        installer_hash = sha256(installer)
        checksums_path = archive.with_name(f"{base}.sha256")
        expected_checksums = (
            f"{archive_hash}  {archive.name}\n"
            f"{installer_hash}  {installer.name}\n"
        )
    else:
        base = archive.name.removesuffix(".tar.gz")
        installer = None
        installer_hash = None
        checksums_path = archive.with_name(f"{base}.sha256")
        expected_checksums = f"{archive_hash}  {archive.name}\n"

    if checksums_path.read_text(encoding="utf-8") != expected_checksums:
        raise ValueError("distribution checksums are missing or changed")

    run_id = f"desktop-package-{time.time_ns()}-{os.getpid()}"
    artifacts = ROOT / "target/e2e-artifacts/package" / run_id
    artifacts.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="meerkat-desktop-pkg-") as temporary:
        root = Path(temporary)
        extracted_root = root / "extracted"
        if is_windows:
            with zipfile.ZipFile(archive) as zipped:
                for info in zipped.infolist():
                    if info.filename.startswith("/") or ".." in Path(info.filename).parts:
                        raise ValueError(f"unsafe archive member: {info.filename}")
                zipped.extractall(extracted_root)
            package_root = extracted_root / base
            binary_map = {
                "meerkat-desktop": "meerkat-desktop.exe",
                "meerkatd": "meerkatd.exe",
                "meerkat-machine": "meerkat-machine.exe",
                "meerkat": "meerkat.exe",
            }
            if "meerkat-agent" in build["binaries"]:
                binary_map["meerkat-agent"] = "meerkat-agent.exe"
            desktop_exe = package_root / "meerkat-desktop.exe"
            required_files = ["LICENSE", "THIRD_PARTY_LICENSES.md"]
        else:
            with tarfile.open(archive) as package:
                for item in package.getmembers():
                    if not (item.isfile() or item.isdir()):
                        raise ValueError("Desktop archive contains a link or special file")
                package.extractall(extracted_root, filter="data")
            package_root = extracted_root / base
            binary_map = {
                "meerkat-desktop": "meerkat-desktop",
                "meerkatd": "meerkatd",
                "meerkat-machine": "meerkat-machine",
                "meerkat": "meerkat",
                "meerkat-agent": "meerkat-agent",
            }
            desktop_exe = package_root / "meerkat-desktop"
            required_files = [
                "LICENSE",
                "THIRD_PARTY_LICENSES.md",
                "install-desktop.sh",
                "meerkat.png",
                "licenses/builtin-agent/LICENSE",
                "licenses/builtin-agent/THIRD_PARTY_NOTICES.md",
            ]

        for relative in required_files:
            if not (package_root / relative).is_file():
                raise ValueError(f"Desktop package missing {relative}")

        for key, filename in binary_map.items():
            path = package_root / filename
            if not path.is_file():
                raise ValueError(f"Desktop package missing binary {filename}")
            if sha256(path) != build["binaries"][key]:
                raise ValueError(f"Packaged binary differs from build provenance: {filename}")

        run_bootstrap_smoke(desktop_exe, root / "home", artifacts)

    if is_windows and args.verify_installer and installer is not None:
        (artifacts / "installer-smoke").mkdir(parents=True, exist_ok=True)
        verify_windows_installer(installer, artifacts, "meerkat-agent" in build["binaries"])

    if sha256(archive) != archive_hash:
        raise ValueError("archive changed during package verification")
    if installer is not None and sha256(installer) != installer_hash:
        raise ValueError("installer changed during package verification")

    evidence = {
        "status": "passed",
        "version": version,
        "cleanup": "passed",
        "archive_sha256": archive_hash,
        "build_sha256": build_hash,
        "classification": "native-extracted-desktop-package",
        "distribution_signing": "none",
    }
    if installer_hash is not None:
        evidence["installer_sha256"] = installer_hash
        evidence["installer_verified"] = bool(args.verify_installer)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(f"[desktop-package] PASS: native extract, bootstrap smoke, and clean shutdown ({archive.name})")


if __name__ == "__main__":
    main()
