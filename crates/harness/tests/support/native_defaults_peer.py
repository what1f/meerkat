#!/usr/bin/env python3
"""Native CLI boundary for the full-access launch business scenarios."""
import json
import os
import sys
from pathlib import Path

record = Path(os.environ["NATIVE_START_RECORD"])
def write(value):
    with record.open("a") as output:
        output.write(json.dumps(value) + "\n")

def emit(value):
    print(json.dumps(value), flush=True)

write({"argv": sys.argv[1:], "cwd": os.getcwd(), "environment": {key: os.environ[key] for key in ("HOME", "PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "LANKIDE_RUNTIME_SOCKET") if key in os.environ}})
if "--version" in sys.argv:
    print("native-fixture")
    sys.exit(0)
if "--help" in sys.argv:
    print("--output-format --input-format --session-id --resume --mcp-config --system-prompt")
    sys.exit(0)
for line in sys.stdin:
    command = json.loads(line)
    write({"command": command})
    if os.environ["NATIVE_ADAPTER"] == "codex":
        method = command.get("method")
        if method == "initialize":
            emit({"id": command["id"], "result": {}})
        elif method in ("thread/start", "thread/resume"):
            emit({"id": command["id"], "result": {"thread": {"id": command["params"].get("threadId", "native-thread")}}})
        elif method == "turn/start":
            emit({"id": command["id"], "result": {"turn": {"id": "native-turn"}}})
            emit({"method": "turn/completed", "params": {"turn": {"id": "native-turn", "status": "completed"}}})
    elif command.get("type") == "user":
        session_flag = "--resume" if "--resume" in sys.argv else "--session-id"
        session = sys.argv[sys.argv.index(session_flag) + 1]
        emit({"type": "result", "session_id": session, "is_error": False, "result": "OK"})
