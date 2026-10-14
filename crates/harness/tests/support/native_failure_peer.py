#!/usr/bin/env python3
import json
import os
import sys

adapter = os.environ["FAILURE_ADAPTER"]
text = os.environ["FAILURE_DETAIL"]
mode = os.environ["FAILURE_MODE"]

def emit(value):
    print(json.dumps(value), flush=True)

def exit_process():
    print(text, file=sys.stderr, flush=True)
    sys.exit(7)

for line in sys.stdin:
    command = json.loads(line)
    if adapter == "pi":
        kind = command.get("type")
        if kind == "get_state":
            if mode == "start_failure":
                emit({"id": command["id"], "type": "response", "success": False, "error": text})
            else:
                emit({"id": command["id"], "type": "response", "success": True, "data": {"sessionId": "failure-session"}})
        elif kind == "prompt":
            emit({"id": command["id"], "type": "response", "success": True})
            if mode == "process_exit":
                exit_process()
            emit({"type": "agent_end", "messages": [{"role": "assistant", "stopReason": "error", "errorMessage": text}]})
    elif adapter == "codex":
        method = command.get("method")
        if method == "initialize":
            emit({"id": command["id"], "result": {}})
        elif method == "thread/start":
            if mode == "start_failure":
                emit({"id": command["id"], "error": {"message": text}})
            else:
                emit({"id": command["id"], "result": {"thread": {"id": "failure-thread"}}})
        elif method == "turn/start":
            emit({"id": command["id"], "result": {"turn": {"id": "failure-turn"}}})
            if mode == "process_exit":
                exit_process()
            emit({"method": "turn/completed", "params": {"turn": {"id": "failure-turn", "status": "failed", "error": {"message": text}}}})
    elif adapter == "claude_code" and command.get("type") == "user":
        if mode == "process_exit":
            exit_process()
        session = sys.argv[sys.argv.index("--session-id") + 1]
        if text.startswith("authentication"):
            emit({"type": "result", "session_id": session, "is_error": True, "subtype": "error_during_execution", "errors": [text]})
        else:
            emit({"type": "result", "session_id": session, "is_error": True, "result": text})
