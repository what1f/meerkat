#!/usr/bin/env python3
import json
import sys

if "--version" in sys.argv:
    print("pi-record-peer")
    sys.exit(0)


def emit(value):
    print(json.dumps(value), flush=True)


for line in sys.stdin:
    command = json.loads(line)
    kind = command.get("type")
    if kind == "get_state":
        emit({"id": command["id"], "type": "response", "command": kind,
              "success": True, "data": {"sessionId": "record-session"}})
    elif kind == "prompt":
        emit({"id": command["id"], "type": "response", "command": kind, "success": True})
        for _ in range(10000):
            emit({"type": "message_update", "assistantMessageEvent": {
                "type": "text_delta", "delta": "streamed text "}})
        for number in range(100):
            emit({"type": "turn_start", "turn": number})
        emit({"type": "tool_execution_update", "partialResult": "streaming tool output"})
        emit({"type": "tool_execution_end", "result": "x" * 10000})
        message = {"role": "assistant", "content": [{"type": "text", "text": "Final retained message"}],
                   "stopReason": "stop"}
        emit({"type": "message_end", "message": message})
        emit({"type": "agent_end", "messages": [message]})
