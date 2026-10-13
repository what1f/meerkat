"""Launch the shipped Desktop entrypoint with real helpers or an external failed helper."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time


def group_members(group):
    rows = subprocess.check_output(["ps", "-eo", "pid=,pgid="], text=True)
    return [int(pid) for pid, pgid in (row.split() for row in rows.splitlines()) if int(pgid) == group]


def run(binary_dir, artifact_dir):
    names = ("lankide-desktop", "lankide-runtime-daemon", "lankide-computer-daemon", "lankide")
    binaries = {name: binary_dir / name for name in names}
    for binary in binaries.values():
        assert binary.is_file() and os.access(binary, os.X_OK), f"build required: {binary}"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    scenarios = []
    for scenario, failed_helper in (
        ("ready", None),
        ("runtime-exits", "lankide-runtime-daemon"),
        ("computer-exits", "lankide-computer-daemon"),
    ):
        result = {"scenario": scenario, "status": "failed"}
        scenarios.append(result)
        with tempfile.TemporaryDirectory(prefix="lk-boot-", dir="/tmp") as directory:
            root = Path(directory)
            home = root / "home"
            bundle = root / "bin"
            bundle.mkdir()
            for name, binary in binaries.items():
                destination = bundle / name
                if name == failed_helper:
                    destination.write_text("#!/bin/sh\nprintf 'PATH=%s\\n' \"$PATH\" >&2\nprintf 'injected helper startup failure\\n' >&2\nexit 42\n")
                    destination.chmod(0o755)
                else:
                    shutil.copy2(binary, destination)
            output = artifact_dir / f"{scenario}.json"
            env = {**os.environ, "LANKIDE_HOME": str(home)}
            env.pop("LANKIDE_RUNTIME_SOCKET", None)
            with (artifact_dir / f"{scenario}.log").open("w") as log:
                process = subprocess.Popen(
                    [str(bundle / "lankide-desktop"), "--local-bootstrap-smoke", "--result", str(output)],
                    env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
                )
                try:
                    code = process.wait(timeout=40)
                    remaining = group_members(process.pid)
                    # Reparented native wrapper children may need a short OS
                    # scheduling interval to observe EOF and finish reaping.
                    deadline = time.monotonic() + 2
                    while remaining and time.monotonic() < deadline:
                        time.sleep(0.05)
                        remaining = group_members(process.pid)
                    result.update(exit_status=code, remaining_pids=remaining)
                    assert not remaining, f"{scenario}: Desktop left owned children alive: {remaining}"
                    assert output.is_file(), f"{scenario}: missing diagnostic result"
                    payload = json.loads(output.read_text())
                    result["bootstrap"] = payload
                    if failed_helper is None:
                        assert code == 0 and payload["status"] == "passed", payload
                        assert payload["runtime_ready"] and payload["computer_connected"] and payload["management"], payload
                    else:
                        assert code != 0 and payload["status"] == "failed", payload
                        assert not payload["computer_connected"], payload
                        assert payload["runtime_ready"] == (failed_helper == "lankide-computer-daemon"), payload
                        helper_log = home / "logs" / f"{failed_helper}.log"
                        helper_output = helper_log.read_text()
                        assert "injected helper startup failure" in helper_output
                    leftovers = [str(path.relative_to(home)) for path in (home / "run").glob("*") if path.is_socket() or path.suffix == ".lock"]
                    assert not leftovers, f"{scenario}: leftover service endpoints: {leftovers}"
                    result.update(status="passed", cleanup="passed")
                finally:
                    # Only this test's new process group can be terminated.
                    remaining = group_members(process.pid)
                    if remaining:
                        os.killpg(process.pid, signal.SIGTERM)
                        try:
                            process.wait(timeout=3)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait()
                    logs = home / "logs"
                    if logs.exists():
                        shutil.copytree(logs, artifact_dir / f"{scenario}-services", dirs_exist_ok=True)
                    (artifact_dir / "result.json").write_text(json.dumps({
                        "status": "passed" if len(scenarios) == 3 and all(item["status"] == "passed" for item in scenarios) else "failed",
                        "classification": "real-process-startup-with-external-faults",
                        "binaries": {name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()} for name, path in binaries.items()},
                        "scenarios": scenarios,
                    }, indent=2) + "\n")
    print(f"Desktop bootstrap E2E: PASS; artifact={artifact_dir / 'result.json'}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary-dir", required=True, type=Path)
    parser.add_argument("--artifact-dir", required=True, type=Path)
    args = parser.parse_args()
    run(args.binary_dir.resolve(), args.artifact_dir.resolve())
