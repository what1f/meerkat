#!/usr/bin/env python3
import json
import os
import sys

mode = os.environ.get("PI_NATIVE_FAULT", "agent_end")
for line in sys.stdin:
    command = json.loads(line)
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
    elif kind == "prompt":
        print(json.dumps({"id": command_id, "type": "response", "command": "prompt", "success": True}), flush=True)
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
