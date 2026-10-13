import json
import sys

first = json.loads(sys.stdin.readline())
if first.get("op") in ("start", "resume"):
    print(json.dumps({"type": "bound", "native_session_id": "pi-scripted-fault"}), flush=True)
for line in sys.stdin:
    command = json.loads(line)
    if command.get("op") == "execute":
        print("not-json", flush=True)
    elif command.get("op") == "close":
        break
