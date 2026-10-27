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

write({"argv": sys.argv[1:], "cwd": os.getcwd(), "environment": {key: os.environ[key] for key in ("HOME", "PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "LANKIDE_RUNTIME_RUN", "ENABLE_TOOL_SEARCH") if key in os.environ}})
if "--version" in sys.argv:
    print("native-fixture")
    sys.exit(0)
if "--help" in sys.argv:
    print("--output-format --input-format --session-id --resume --mcp-config --system-prompt")
    sys.exit(0)
if os.environ["NATIVE_ADAPTER"] == "claude_code":
    import urllib.request
    import http.client
    config=json.loads(Path(sys.argv[sys.argv.index("--mcp-config")+1]).read_text())
    server = config["mcpServers"]["lankide"]
    # Reject unauthenticated requests before decoding or waiting for a body.
    import socket
    from urllib.parse import urlparse
    address = urlparse(server["url"])
    for authorization in ("", "Authorization: Bearer wrong-token\r\n"):
        with socket.create_connection((address.hostname, address.port), timeout=2) as connection:
            connection.sendall(("POST /mcp HTTP/1.1\r\nHost: localhost\r\n" + authorization + "Content-Length: 100\r\n\r\n").encode())
            assert connection.recv(4096).startswith(b"HTTP/1.1 401 Unauthorized")
    headers = {"Content-Type": "application/json", "aUtHoRiZaTiOn": server["headers"]["Authorization"]}
    connection = http.client.HTTPConnection(address.hostname, address.port, timeout=2)
    connection.request("POST", "/mcp", body=json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/list"}), headers=headers)
    response = connection.getresponse()
    assert response.status == 200
    write({"mcp_tools":json.load(response)["result"]["tools"]})
    connection.close()
for line in sys.stdin:
    command = json.loads(line)
    write({"command": command})
    if os.environ["NATIVE_ADAPTER"] == "codex":
        method = command.get("method")
        if method == "initialize":
            emit({"id": command["id"], "result": {}})
        elif method in ("thread/start", "thread/resume"):
            for spec in command["params"].get("dynamicTools", []):
                if spec["type"] == "namespace":
                    assert spec["name"] and spec["tools"]
                else:
                    assert not spec.get("deferLoading"), "deferred tool requires namespace"
            emit({"id": command["id"], "result": {"thread": {"id": command["params"].get("threadId", "native-thread")}}})
        elif method == "turn/start":
            emit({"id": command["id"], "result": {"turn": {"id": "native-turn"}}})
            emit({"id": "namespace-call", "method": "item/tool/call", "params": {"threadId": "native-thread", "turnId": "native-turn", "callId": "report-call", "namespace": "lankide", "tool": "report", "arguments": {"value": "namespace-roundtrip"}}})
        elif command.get("id") == "namespace-call":
            assert command["result"]["success"] is True
            assert "namespace-roundtrip" in command["result"]["contentItems"][0]["text"]
            emit({"method": "turn/completed", "params": {"turn": {"id": "native-turn", "status": "completed"}}})
    elif command.get("type") == "user":
        request = urllib.request.Request(server["url"], data=json.dumps({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"report","arguments":{"value":"namespace-roundtrip"}}}).encode(), headers=headers)
        with urllib.request.urlopen(request) as response:
            result = json.load(response)["result"]
        assert result["isError"] is True
        assert result["content"][0]["text"] == "Access to this conversation was denied."
        assert result["structuredContent"] == {"code":"permission_denied", "retryable":False}
        session_flag = "--resume" if "--resume" in sys.argv else "--session-id"
        session = sys.argv[sys.argv.index(session_flag) + 1]
        emit({"type": "result", "session_id": session, "is_error": False, "result": "OK"})
