"""Reopen Desktop over real old helpers while a cold Harness probe is still running."""

from contextlib import contextmanager
import argparse
import json
import os
import plistlib
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import threading
import time
from urllib.request import Request, urlopen


def catalog(home):
    origin = (home / "run/management.url").read_text().strip()
    request = Request(origin + "/api/v1/get_machine_catalog", data=b'{"request_id":"update-e2e","workspace":null}',
                      headers={"Content-Type": "application/json"})
    with urlopen(request, timeout=1) as response:
        return json.load(response)


@contextmanager
def running_update(binaries, artifact_dir, delay=8):
    with tempfile.TemporaryDirectory(prefix="lk-update-", dir="/tmp") as directory:
        root = Path(directory)
        home = root / "home"
        home.mkdir()
        (home / "runtime.json").write_text(json.dumps({
            "machine_link_listen": "127.0.0.1:0", "remote_access_port": 0,
        }))
        contents = root / "Boot Update E2E.app/Contents"
        bundle = contents / "MacOS"
        bundle.mkdir(parents=True)
        (contents / "Info.plist").write_bytes(plistlib.dumps({
            "CFBundleIdentifier": "com.meerkat.boot-update-e2e",
            "CFBundleName": "Meerkat Boot Update E2E",
            "CFBundleExecutable": "meerkat-desktop",
            "CFBundlePackageType": "APPL",
            "NSAppTransportSecurity": {"NSAllowsLocalNetworking": True},
        }))
        for name, binary in binaries.items():
            shutil.copy2(binary, bundle / name)
        harnesses = root / "harnesses"
        harnesses.mkdir()
        # Only the external Harness is scripted. Runtime, Machine and Desktop
        # are the product executables, with their normal discovery and pairing.
        pi = harnesses / "pi"
        pi.write_text(f'''#!{shutil.which("python3")}
import json, pathlib, sys, time
marker = pathlib.Path({str(root / "cold-probe")!r})
if "--version" in sys.argv:
    if marker.exists():
        delay = float(marker.read_text())
        marker.unlink()
        time.sleep(delay)
    print("pi update-e2e")
else:
    for line in sys.stdin:
        request = json.loads(line)
        print(json.dumps({{"id": request.get("id"), "type": "response", "success": True,
                          "data": {{"models": []}}}}), flush=True)
''')
        pi.chmod(0o755)
        for name in ("codex", "claude"):
            path = harnesses / name
            path.write_text("#!/bin/sh\nexit 1\n")
            path.chmod(0o755)
        env = {**os.environ, "HOME": str(root), "MEERKAT_HOME": str(home),
               "SHELL": "/bin/sh", "PATH": f"{harnesses}:/usr/bin:/bin"}
        env.pop("MEERKAT_RUNTIME_RUN", None)
        owners = []
        with (artifact_dir / "update.log").open("w") as log:
            try:
                for name in ("meerkatd", "meerkat-machine"):
                    owner = subprocess.Popen([str(bundle / name), "--home", str(home)],
                                             env=env, stdout=log, stderr=subprocess.STDOUT,
                                             start_new_session=True)
                    owners.append(owner)
                    # Reap the original owners as launchd does, so kill -0 does
                    # not mistake an exited child for a live service.
                    threading.Thread(target=owner.wait, daemon=True).start()
                deadline = time.monotonic() + 20
                while True:
                    assert all(owner.poll() is None for owner in owners), "old helper exited"
                    try:
                        if any(item["is_local"] and item["observed_state"] == "running"
                               for item in catalog(home)["machines"]):
                            break
                    except (OSError, ValueError):
                        pass
                    assert time.monotonic() < deadline, "old Machine did not connect"
                    time.sleep(0.05)
                old_url = (home / "run/management.url").read_text().strip()
                # This is the update criterion used by the shipped shell:
                # replace the helpers at the same paths with newer builds.
                for name in ("meerkatd", "meerkat-machine"):
                    staged = bundle / f".{name}.update"
                    shutil.copyfile(binaries[name], staged)
                    staged.chmod(0o755)
                    staged.replace(bundle / name)
                (root / "cold-probe").write_text(str(delay))
                yield root, home, bundle, env, owners, old_url, log
            finally:
                for owner in reversed(owners):
                    if owner.poll() is None:
                        os.killpg(owner.pid, signal.SIGTERM)
                        owner.wait(timeout=10)
                if (home / "logs").exists():
                    shutil.copytree(home / "logs", artifact_dir / "update-services", dirs_exist_ok=True)


def replace_running_services(binaries, artifact_dir):
    result = {"scenario": "update-with-cold-harness", "status": "failed"}
    with running_update(binaries, artifact_dir) as (root, home, bundle, env, owners, old_url, log):
        output = artifact_dir / "update-bootstrap.json"
        started = time.monotonic()
        process = subprocess.Popen(
            [str(bundle / "meerkat-desktop"), "--local-bootstrap-smoke", "--result", str(output)],
            env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
        )
        try:
            discovered_url = None
            while process.poll() is None:
                try:
                    discovered_url = (home / "run/management.url").read_text().strip()
                except FileNotFoundError:
                    pass
                assert time.monotonic() - started < 90, "Desktop replacement timed out"
                time.sleep(0.01)
            assert process.wait() == 0, "first Desktop launch failed during helper replacement"
            payload = json.loads(output.read_text())
            assert payload["status"] == "passed" and payload["machine_connected"], payload
            assert discovered_url and payload["navigation_url"] == discovered_url + "/", payload
            assert all(owner.poll() == 0 for owner in owners), "old helpers were not replaced cleanly"
            assert time.monotonic() - started >= 8, "cold Harness was not exercised"
            assert not list((home / "run").glob("*.lock")), "replacement helpers were not cleaned up"
            result.update(status="passed", cleanup="passed", bootstrap=payload,
                          old_url=old_url, elapsed_seconds=round(time.monotonic() - started, 2))
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
    return result


def review_window(binaries, artifact_dir):
    """Stage the same update for an actual-window Machine Use review.

    This fixture does not accept the UI. The reviewer must record the loader
    after six seconds, management navigation without Retry, and normal Quit.
    """
    with running_update(binaries, artifact_dir, delay=20) as (root, home, bundle, env, owners, old_url, log):
        process = subprocess.Popen([str(bundle / "meerkat-desktop")], env=env,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        session = {"app": str(bundle.parent.parent), "home": str(home), "pid": process.pid,
                   "old_pids": [owner.pid for owner in owners], "old_url": old_url}
        (artifact_dir / "session.json").write_text(json.dumps(session, indent=2) + "\n")
        print(json.dumps(session), flush=True)
        try:
            assert process.wait(timeout=180) == 0, "test window did not quit normally"
            assert not list((home / "run").glob("*.lock")), "test window left services running"
            (artifact_dir / "cleanup.json").write_text(json.dumps({
                "desktop_exit": process.returncode, "remaining_locks": [],
                "old_helpers_exited": all(owner.poll() == 0 for owner in owners),
            }) + "\n")
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary-dir", required=True, type=Path)
    parser.add_argument("--artifact-dir", required=True, type=Path)
    parser.add_argument("--window", action="store_true", help="prepare a real window for Machine Use review")
    args = parser.parse_args()
    args.artifact_dir.mkdir(parents=True, exist_ok=True)
    binaries = {name: args.binary_dir.resolve() / name
                for name in ("meerkat-desktop", "meerkatd", "meerkat-machine", "meerkat")}
    if args.window:
        review_window(binaries, args.artifact_dir)
    else:
        result = replace_running_services(binaries, args.artifact_dir)
        (args.artifact_dir / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result))
