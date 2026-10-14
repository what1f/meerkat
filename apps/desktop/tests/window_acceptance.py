"""Validate an explicit human/Computer Use review of the actual product window.

This is an evidence gate, not UI automation. It never manufactures a passing
review from OCR, a bootstrap response, or a test-only composition.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
SCENARIOS = (
    "first-launch-local-computer", "computer-list-offline", "pairing-code-dialog",
    "empty-state", "agent-list", "agent-list-hover", "feishu-create-agent",
    "telegram-create-agent", "edit-agent", "edit-agent-reaction", "edit-agent-advanced",
    "workspace-settings", "create-workspace", "create-workspace-provider", "agent-onboarding",
    "settings", "dark-agent-list", "dark-create-workspace", "dark-edit-agent",
    "dark-workspace-settings", "dark-settings", "real-profile-images",
)
BINARY_NAMES = ("lankide-desktop", "lankide-runtime-daemon", "lankide-computer-daemon", "lankide")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def binary_paths():
    requested = os.environ.get("LANKIDE_VISUAL_BINARY")
    if os.environ.get("LANKIDE_RELEASE_REQUIRED") == "1" and not requested:
        raise ValueError("release gate requires LANKIDE_VISUAL_BINARY")
    desktop = Path(requested) if requested else ROOT / "target/debug/lankide-desktop"
    directory = Path(os.environ.get("LANKIDE_RELEASE_BIN_DIR", str(desktop.parent)))
    paths = {name: directory / name for name in BINARY_NAMES}
    paths["lankide-desktop"] = desktop
    for path in paths.values():
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError(f"product executable missing: {path}")
    return paths


def inputs(paths):
    # Reuse the release snapshot algorithm rather than introducing an mtime gate.
    import runpy
    provenance = runpy.run_path(str(ROOT / "scripts/release-provenance"))
    return {
        "sources": provenance["source_snapshot"](),
        "binaries": {name: digest(path) for name, path in paths.items()},
        "baselines": {str(path.relative_to(ROOT)): digest(path) for path in sorted((ROOT / "docs/assets/refactor-ui").glob("*.png"))},
    }


def attachment(review_dir, value):
    path = (review_dir / value["path"]).resolve()
    if not path.is_relative_to(review_dir.resolve()) or not path.is_file():
        raise ValueError("review attachment is missing or outside the review directory")
    if digest(path) != value["sha256"]:
        raise ValueError(f"review attachment changed: {path.name}")
    return path


def screenshot_size(data):
    # Computer Use may return JPEG. Preserve the original bytes instead of
    # relabelling them as PNG or requiring a lossy re-encode of evidence.
    if data.startswith(b"\x89PNG\r\n\x1a\n") and len(data) >= 24 and data[12:16] == b"IHDR":
        return struct.unpack(">II", data[16:24])
    if data.startswith(b"\xff\xd8") and data.endswith(b"\xff\xd9"):
        offset = 2
        while offset + 4 < len(data):
            if data[offset] != 0xff:
                break
            while offset < len(data) and data[offset] == 0xff:
                offset += 1
            if offset >= len(data):
                break
            marker = data[offset]
            offset += 1
            if marker in (0xd8, 0xd9, 0x01) or 0xd0 <= marker <= 0xd7:
                continue
            if marker == 0xda or offset + 2 > len(data):
                break
            length = int.from_bytes(data[offset:offset + 2], "big")
            if length < 2 or offset + length > len(data):
                break
            if marker in (0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf):
                if length < 8:
                    break
                height, width = struct.unpack(">HH", data[offset + 3:offset + 7])
                return width, height
            offset += length
    raise ValueError("review screenshot has an invalid PNG or JPEG header")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", type=Path, help="write a pending review template; does not launch or accept the app")
    parser.add_argument("--review", type=Path, default=os.environ.get("LANKIDE_VISUAL_REVIEW"))
    args = parser.parse_args()
    if os.environ.get("LANKIDE_VISUAL_SCENARIOS"):
        raise ValueError("partial scenario selection cannot satisfy Desktop acceptance")
    paths = binary_paths()
    if args.prepare:
        args.prepare.mkdir(parents=True, exist_ok=False)
        review = {
            "classification": "actual-window-reviewed", "status": "pending", "reviewer": "",
            "inputs": inputs(paths), "binary_paths": {name: str(path.resolve()) for name, path in paths.items()},
            "runtime_home": "", "owned_processes": [], "cleanup": "pending",
            "scenarios": [{"id": name, "status": "pending", "observations": "", "screenshot": {}, "actions": {}} for name in SCENARIOS],
        }
        (args.prepare / "review.json").write_text(json.dumps(review, indent=2) + "\n")
        print(f"Pending review template: {args.prepare / 'review.json'}; follow apps/desktop/tests/window-acceptance.md")
        return
    if args.review is None:
        print("PREREQUISITE_MISSING: supply LANKIDE_VISUAL_REVIEW after reviewing the actual product window", file=sys.stderr)
        raise SystemExit(4)
    review_path = Path(args.review).resolve()
    review = json.loads(review_path.read_text())
    if review.get("classification") != "actual-window-reviewed" or review.get("status") != "passed" or not review.get("reviewer", "").strip():
        raise ValueError("a completed actual-window review is required")
    if review["inputs"] != inputs(paths):
        raise ValueError("review source, binaries or baseline content differs from this gate")
    scenarios = review["scenarios"]
    if sorted(item["id"] for item in scenarios) != sorted(SCENARIOS):
        raise ValueError("review must contain the full required scenario set exactly once")
    for scenario in scenarios:
        if scenario["status"] != "passed" or not scenario["observations"].strip():
            raise ValueError(f"scenario not reviewed: {scenario['id']}")
        screenshot = attachment(review_path.parent, scenario["screenshot"]).read_bytes()
        width, height = screenshot_size(screenshot)
        if width < 900 or height < 600:
            raise ValueError("review screenshot does not show the complete product window")
        if not attachment(review_path.parent, scenario["actions"]).read_text().strip():
            raise ValueError("scenario action/evidence log is empty")
    owned = review["owned_processes"]
    if not {"desktop", "runtime", "computer"}.issubset({item["role"] for item in owned}) or review["cleanup"] != "passed":
        raise ValueError("owned process cleanup was not reviewed")
    for item in owned:
        pid = int(item["pid"])
        if pid <= 1 or not item["started_at"].strip():
            raise ValueError("invalid owned process identity")
        current = subprocess.run(["ps", "-p", str(pid), "-o", "lstart="], capture_output=True, text=True)
        if current.returncode == 0 and current.stdout.strip() == item["started_at"].strip():
            raise ValueError(f"review process is still alive: {item['role']}")
    home = Path(review["runtime_home"])
    if not home.is_absolute() or not home.name or home == Path.home() / ".lankide":
        raise ValueError("review must identify its isolated Runtime home")
    for name in ("runtime.sock", "runtime.lock", "computer.lock"):
        if (home / "run" / name).exists():
            raise ValueError(f"review left a service endpoint: {name}")
    result = {"status": "passed", "classification": "actual-window-reviewed", "review": str(review_path),
              "review_sha256": digest(review_path), "scenario_count": len(SCENARIOS), "cleanup": "passed"}
    output = review_path.parent / "result.json"
    output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Desktop actual-window review: PASS; artifact={output}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise SystemExit(f"Desktop acceptance failed: {error}")
