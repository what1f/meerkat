#!/usr/bin/env python3
import json
import os
import sys

record = os.environ.get("NATIVE_START_RECORD")
def write_record(value):
    if record:
        with open(record, "a") as output:
            output.write(json.dumps(value) + "\n")

write_record({"argv": sys.argv[1:], "cwd": os.getcwd(), "environment": {key: os.environ[key] for key in ("HOME", "PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "LANKIDE_RUNTIME_SOCKET") if key in os.environ}})
if "--version" in sys.argv:
    print("native-fixture")
    sys.exit(0)

mode = os.environ.get("PI_NATIVE_FAULT", "agent_end")
for line in sys.stdin:
    command = json.loads(line)
    write_record({"command": command})
    kind = command.get("type")
    command_id = command.get("id")
    if kind == "get_state":
        print(json.dumps({
            "id": command_id,
            "type": "response",
            "command": "get_state",
            "success": True,
            "data": {"sessionId": f"pi-native-{mode}", "sessionFile": None}
        }), flush=True)
    elif kind == "get_available_models":
        print(json.dumps({"id": command_id, "type": "response", "success": True, "data": {"models": [{"id": "native-model", "name": "Native model", "provider": "fixture"}]}}), flush=True)
    elif kind == "prompt":
        print(json.dumps({"id": command_id, "type": "response", "command": "prompt", "success": True}), flush=True)
        if mode == "success":
            print(json.dumps({"type": "agent_end", "messages": [{"role": "assistant", "content": [{"type": "text", "text": "OK"}], "stopReason": "stop"}]}), flush=True)
    elif kind == "steer":
        print(json.dumps({"id": command_id, "type": "response", "command": "steer", "success": True}), flush=True)
    elif kind == "abort" and mode == "agent_end":
        print(json.dumps({
            "type": "agent_end",
            "messages": [{
                "role": "assistant",
                "content": [],
                "stopReason": "aborted",
                "errorMessage": "Request was aborted"
            }],
            "willRetry": False
        }), flush=True)
    elif kind == "abort":
        pass
