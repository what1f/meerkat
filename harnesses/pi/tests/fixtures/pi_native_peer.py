#!/usr/bin/env python3
import json
import os
import sys

record = os.environ.get("NATIVE_START_RECORD")
def write_record(value):
    if record:
        with open(record, "a") as output:
            output.write(json.dumps(value) + "\n")

write_record({"argv": sys.argv[1:], "cwd": os.getcwd(), "environment": {key: os.environ[key] for key in ("HOME", "PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "MEERKAT_AGENT_SOCKET") if key in os.environ}})
if "--version" in sys.argv:
    print(os.environ.get("PI_TEST_VERSION", "native-fixture"))
    sys.exit(0)

if "--extension" in sys.argv and os.environ.get("PI_TEST_VERSION"):
    import subprocess, base64
    from pathlib import Path
    source=Path(sys.argv[sys.argv.index("--extension")+1]).read_bytes()
    code="""
const tools=[], handlers=[], active=['read']; let invoked;
const pi={registerTool:t=>tools.push(t),on:(name,fn)=>handlers.push(fn),getActiveTools:()=>active,setActiveTools:values=>{active.splice(0,active.length,...values)}};
const module=await import('data:text/javascript;base64,SOURCE');module.default(pi);
for (const handler of handlers) await handler();
const wrapper=tools.find(t=>t.name==='meerkat_tools');let discovered,described;
if(wrapper){const context={ui:{input:async (_title,payload)=>{invoked=JSON.parse(payload);return JSON.stringify({success:true,structured_content:{ok:true}})}}};
 discovered=JSON.parse((await wrapper.execute('list',{argv:['list']},null,null,context)).content[0].text);
 described=JSON.parse((await wrapper.execute('describe',{argv:['describe','report']},null,null,context)).content[0].text);
 await wrapper.execute('call',{argv:['call','report','{"value":1}']},null,null,context);}
console.log(JSON.stringify({extension_tools:tools.map(t=>({name:t.name,exposure:t.exposure,parameters:t.parameters})),active,discovered,described,invoked}));
""".replace('SOURCE',base64.b64encode(source).decode())
    write_record(json.loads(subprocess.check_output(['node','--input-type=module','-e',code],text=True)))

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
