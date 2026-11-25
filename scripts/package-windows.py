#!/usr/bin/env python3
"""Package the Windows Desktop portable zip and NSIS installer from a verified release build."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parent.parent
ARCH_BY_TARGET = {
    "x86_64-pc-windows-msvc": "x86_64",
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def zip_directory(source_dir: Path, root_name: str, destination: Path) -> None:
    if destination.exists():
        destination.unlink()
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as zipped:
        for path in sorted(source_dir.rglob("*")):
            if path.is_symlink():
                raise ValueError(f"Windows package cannot contain symlink: {path}")
            if path.is_file():
                arcname = (Path(root_name) / path.relative_to(source_dir)).as_posix()
                zipped.write(path, arcname)


def write_nsis_manifests(stage: Path, install_list: Path, uninstall_list: Path) -> None:
    files = sorted(path for path in stage.rglob("*") if path.is_file())
    dirs = sorted(
        {path.parent.relative_to(stage) for path in files if path.parent != stage},
        key=lambda rel: (len(rel.parts), rel.as_posix()),
    )

    install_lines = []
    grouped: dict[Path, list[Path]] = {}
    for file_path in files:
        rel_dir = file_path.parent.relative_to(stage)
        grouped.setdefault(rel_dir, []).append(file_path)
    for rel_dir in sorted(grouped.keys(), key=lambda rel: (len(rel.parts), rel.as_posix())):
        win_dir = "$INSTDIR" if rel_dir == Path(".") else "$INSTDIR\\" + "\\".join(rel_dir.parts)
        install_lines.append(f'SetOutPath "{win_dir}"')
        for file_path in grouped[rel_dir]:
            install_lines.append(f'File "{file_path}"')
    install_list.write_text("\n".join(install_lines) + "\n", encoding="utf-8")

    uninstall_lines = []
    for file_path in files:
        rel = file_path.relative_to(stage)
        win_path = "$INSTDIR\\" + "\\".join(rel.parts)
        uninstall_lines.append(f'Delete "{win_path}"')
    for rel_dir in reversed(dirs):
        win_dir = "$INSTDIR\\" + "\\".join(rel_dir.parts)
        uninstall_lines.append(f'RMDir "{win_dir}"')
    uninstall_list.write_text("\n".join(uninstall_lines) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", nargs="?", default="x86_64-pc-windows-msvc")
    args = parser.parse_args()

    arch = ARCH_BY_TARGET.get(args.target)
    if arch is None:
        raise SystemExit(f"Unsupported Windows target: {args.target}")

    subprocess.run([sys.executable, str(ROOT / "scripts/verify-production-boundary")], check=True)

    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target")))
    default_bin_dir = target_dir / args.target / "release"
    if not default_bin_dir.is_dir():
        default_bin_dir = target_dir / "release"
    release_dir = Path(os.environ.get("MEERKAT_RELEASE_BIN_DIR", str(default_bin_dir))).resolve()
    output_dir = Path(os.environ.get("MEERKAT_PACKAGE_OUTPUT_DIR", str(release_dir))).resolve()
    provenance = Path(os.environ.get("MEERKAT_RELEASE_PROVENANCE", str(release_dir / "build-provenance.json")))

    subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts/release-provenance"),
            "verify",
            "--binary-dir",
            str(release_dir),
            "--output",
            str(provenance),
        ],
        check=True,
    )

    version = tomllib.loads((ROOT / "apps/desktop/Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    base = f"Meerkat-{version}-windows-{arch}"

    # Keep meerkat-desktop.exe distinct from meerkat.exe on case-insensitive Windows filesystems.
    required_binaries = [
        "meerkat-desktop.exe",
        "meerkatd.exe",
        "meerkat-machine.exe",
        "meerkat.exe",
    ]
    for name in required_binaries:
        if not (release_dir / name).is_file():
            raise SystemExit(f"release binary is missing: {release_dir / name}")

    makensis = shutil.which("makensis")
    if makensis is None:
        for candidate in (
            Path(r"C:\Program Files (x86)\NSIS\makensis.exe"),
            Path(r"C:\Program Files\NSIS\makensis.exe"),
        ):
            if candidate.is_file():
                makensis = str(candidate)
                break
    if makensis is None:
        raise SystemExit("makensis (NSIS) is required to build the Windows installer")

    output_dir.mkdir(parents=True, exist_ok=True)
    archive = output_dir / f"{base}.zip"
    installer = output_dir / f"{base}-setup.exe"

    with tempfile.TemporaryDirectory(prefix="meerkat-win-pkg-") as temporary:
        stage = Path(temporary) / base
        stage.mkdir(parents=True)
        for name in required_binaries:
            shutil.copy2(release_dir / name, stage / name)

        shutil.copy2(ROOT / "LICENSE", stage / "LICENSE")
        shutil.copy2(ROOT / "THIRD_PARTY_LICENSES.md", stage / "THIRD_PARTY_LICENSES.md")
        shutil.copytree(ROOT / "licenses", stage / "licenses")

        zip_directory(stage, base, archive)

        install_list = Path(temporary) / "install-files.nsh"
        uninstall_list = Path(temporary) / "uninstall-files.nsh"
        write_nsis_manifests(stage, install_list, uninstall_list)

        stop_binaries = list(required_binaries)

        if installer.exists():
            installer.unlink()
        subprocess.run(
            [
                makensis,
                "/V2",
                f"/DVERSION={version}",
                f"/DSOURCE_DIR={stage}",
                f"/DICON_PATH={ROOT / 'apps/desktop/assets/app-icon.ico'}",
                f"/DINSTALL_LIST={install_list}",
                f"/DUNINSTALL_LIST={uninstall_list}",
                f"/DSTOP_BINARIES={','.join(stop_binaries)}",
                f"/DOUTPUT_FILE={installer}",
                str(ROOT / "scripts/windows-installer.nsi"),
            ],
            check=True,
        )

    checksums = output_dir / f"{base}.sha256"
    checksums.write_text(
        "".join(f"{sha256(path)}  {path.name}\n" for path in (archive, installer)),
        encoding="utf-8",
    )
    print(f"Package ready: {archive}")
    print(f"Installer ready: {installer}")


if __name__ == "__main__":
    main()
