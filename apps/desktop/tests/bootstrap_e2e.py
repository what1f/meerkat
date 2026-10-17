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
from urllib.parse import urlsplit


def assert_navigation(payload, discovered_url):
    assert discovered_url, "Runtime did not publish a discovery URL"
    discovered = urlsplit(discovered_url.strip())
    expected = f"{discovered.scheme}://{discovered.netloc}/"
    assert payload["navigation_url"] == expected, payload
    assert payload["navigation_mode"] == "headless-intended", payload


def wait_for_smoke(process, home, timeout=40):
    deadline = time.monotonic() + timeout
    discovered_url = None
    while process.poll() is None:
        try:
            value = (home / "run/management.url").read_text().strip()
            if value:
                discovered_url = value
        except FileNotFoundError:
            pass
        if time.monotonic() >= deadline:
            raise subprocess.TimeoutExpired(process.args, timeout)
        time.sleep(0.01)
    return process.wait(), discovered_url


def group_members(group):
    rows = subprocess.check_output(["ps", "-eo", "pid=,pgid="], text=True)
    return [int(pid) for pid, pgid in (row.split() for row in rows.splitlines()) if int(pgid) == group]


def reuse_running_services(binaries, artifact_dir):
    result = {"scenario": "unbundled-default-home-existing-owner", "status": "failed"}
    with tempfile.TemporaryDirectory(prefix="lk-owner-", dir="/tmp") as directory:
        home = Path(directory) / ".lankide"
        home.mkdir()
        (home / "runtime.json").write_text(json.dumps({"machine_link_listen": "127.0.0.1:0", "remote_access_port": 0}))
        env = {**os.environ, "HOME": directory}
        env.pop("LANKIDE_HOME", None)
        env.pop("LANKIDE_RUNTIME_RUN", None)
        owners = []
        with (artifact_dir / "existing-owner.log").open("w") as log:
            try:
                for name in ("lankided", "lankide-machine"):
                    owners.append(subprocess.Popen(
                        [str(binaries[name]), "--home", str(home)],
                        env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
                    ))
                deadline = time.monotonic() + 10
                while True:
                    assert all(owner.poll() is None for owner in owners), "service owner exited"
                    assert time.monotonic() < deadline, "service owner did not start"
                    probe = subprocess.run(
                        [str(binaries["lankide-desktop"]), "--local-bootstrap-smoke", "--result", str(artifact_dir / "existing-owner-ready.json")],
                        env={**env, "LANKIDE_RUNTIME_RUN": str(home / "run")},
                        stdout=log, stderr=subprocess.STDOUT, timeout=10,
                    )
                    if probe.returncode == 0:
                        break
                    time.sleep(0.05)
                locks = {name: (home / f"run/{name}.lock").read_bytes() for name in ("runtime", "machine")}
                for attempt in range(2):
                    output = artifact_dir / f"existing-owner-{attempt}.json"
                    process = subprocess.Popen(
                        [str(binaries["lankide-desktop"]), "--local-bootstrap-smoke", "--result", str(output)],
                        env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
                    )
                    try:
                        assert process.wait(timeout=40) == 0, "Desktop could not reuse services"
                        payload = json.loads(output.read_text())
                        assert payload["runtime_ready"] and payload["machine_connected"] and payload["management"], payload
                        assert_navigation(payload, (home / "run/management.url").read_text())
                        assert not group_members(process.pid), "Desktop left a helper alive"
                        assert all(owner.poll() is None for owner in owners), "Desktop stopped an external owner"
                        for name, contents in locks.items():
                            assert (home / f"run/{name}.lock").read_bytes() == contents, "Desktop replaced a service owner"
                    finally:
                        if process.poll() is None:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait()
                result.update(status="passed", cleanup="passed")
            finally:
                for owner in reversed(owners):
                    if owner.poll() is None:
                        os.killpg(owner.pid, signal.SIGTERM)
                        try:
                            owner.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(owner.pid, signal.SIGKILL)
                            owner.wait()
                    assert not group_members(owner.pid), "service owner cleanup failed"
                assert not list((home / "run").glob("*.lock")), "service ownership remained after shutdown"
    return result


def run(binary_dir, artifact_dir):
    names = ("lankide-desktop", "lankided", "lankide-machine", "lankide")
    binaries = {name: binary_dir / name for name in names}
    for binary in binaries.values():
        assert binary.is_file() and os.access(binary, os.X_OK), f"build required: {binary}"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    scenarios = [reuse_running_services(binaries, artifact_dir)]
    for scenario, failed_helper in (
        ("ready", None),
        ("unbundled-default-home-ready", None),
        ("runtime-exits", "lankided"),
        ("machine-exits", "lankide-machine"),
        ("helper-missing", "missing:lankided"),
    ):
        result = {"scenario": scenario, "status": "failed"}
        scenarios.append(result)
        with tempfile.TemporaryDirectory(prefix="lk-boot-", dir="/tmp") as directory:
            root = Path(directory)
            default_home = scenario == "unbundled-default-home-ready"
            home = root / (".lankide" if default_home else "home")
            home.mkdir()
            (home / "runtime.json").write_text(json.dumps({"machine_link_listen": "127.0.0.1:0", "remote_access_port": 0}))
            bundle = root / "bin"
            bundle.mkdir()
            for name, binary in binaries.items():
                destination = bundle / name
                if failed_helper == f"missing:{name}":
                    continue
                if name == failed_helper:
                    destination.write_text("#!/bin/sh\nprintf 'PATH=%s\\n' \"$PATH\" >&2\nprintf 'injected helper startup failure\\n' >&2\nexit 42\n")
                    destination.chmod(0o755)
                else:
                    shutil.copy2(binary, destination)
            output = artifact_dir / f"{scenario}.json"
            env = {**os.environ, "HOME": str(root)}
            env.pop("LANKIDE_HOME", None)
            if not default_home:
                env["LANKIDE_HOME"] = str(home)
            env.pop("LANKIDE_RUNTIME_RUN", None)
            with (artifact_dir / f"{scenario}.log").open("w") as log:
                process = subprocess.Popen(
                    [str(bundle / "lankide-desktop"), "--local-bootstrap-smoke", "--result", str(output)],
                    env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
                )
                try:
                    code, discovered_url = wait_for_smoke(process, home)
                    remaining = group_members(process.pid)
                    # Reparented native wrapper children may need a short OS
                    # scheduling interval to observe EOF and finish reaping.
                    deadline = time.monotonic() + 2
                    while remaining and time.monotonic() < deadline:
                        time.sleep(0.05)
                        remaining = group_members(process.pid)
                    result.update(exit_status=code, remaining_pids=remaining, discovered_url=discovered_url)
                    assert not remaining, f"{scenario}: Desktop left owned children alive: {remaining}"
                    assert output.is_file(), f"{scenario}: missing diagnostic result"
                    payload = json.loads(output.read_text())
                    result["bootstrap"] = payload
                    if failed_helper is None:
                        assert code == 0 and payload["status"] == "passed", payload
                        assert payload["runtime_ready"] and payload["machine_connected"] and payload["management"], payload
                        assert_navigation(payload, discovered_url)
                        credentials = json.loads((home / "machine-credentials.json").read_text())
                        assert credentials["local_discovery"] and credentials["machine_id"] != "local", "first launch did not self-pair"
                        assert payload["local_machine_id"] == credentials["machine_id"], "Desktop waited for another Machine"
                        for name in ("lankided", "lankide-machine"):
                            assert (home / "logs" / f"{name}.log").is_file(), "Desktop did not start its helper"
                    else:
                        assert code != 0 and payload["status"] == "failed", payload
                        assert not payload["machine_connected"], payload
                        assert payload["runtime_ready"] == (failed_helper == "lankide-machine"), payload
                        # A helper that fails before logger setup still reports
                        # its failure through the inherited foreground stream.
                        helper_output = (artifact_dir / f"{scenario}.log").read_text()
                        if scenario == "helper-missing":
                            assert payload["error_code"] == "helper-missing", payload
                        else:
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
                        "status": "passed" if len(scenarios) == 6 and all(item["status"] == "passed" for item in scenarios) else "failed",
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
