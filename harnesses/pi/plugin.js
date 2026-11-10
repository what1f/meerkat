"use strict";
function prepare(request) {
  const paths = prepareNative(request, {
    sourceVariable: "PI_CODING_AGENT_DIR",
    sourceRelative: ".pi/agent",
    files: ["auth.json", "models.json", "settings.json"],
    resources: ["extensions", "skills", "prompts", "SYSTEM.md"],
    pathKeys: ["extensions", "skills", "prompts", "themes"],
  });
  // A user shell prefix may rewrite PATH; keep Meerkat's commands in front.
  const settingsPath = paths.directory + "/settings.json";
  if (request.config.environment.MEERKAT_PATH && ctx.exists(settingsPath)) {
    const settings = JSON.parse(ctx.read(settingsPath));
    if (settings.shellCommandPrefix) {
      settings.shellCommandPrefix += '\nexport PATH="$MEERKAT_PATH:$PATH"';
      ctx.write(settingsPath, JSON.stringify(settings, null, 2));
    }
  }
  return paths;
}
let request,
  paths,
  processId,
  nativeId,
  active = false,
  output = "",
  nextId = 0,
  cancelReason = null,
  cancelAt = 0,
  contextTokens = 0,
  // Pi compacts near the full window; Meerkat asks for it earlier, after the turn.
  compaction = null;
const toolNames = new Map(),
  observed = new Set(),
  uiRequests = new Map(),
  results = new Map();
function send(type, fields = {}) {
  const id = "meerkat-" + ++nextId;
  ctx.send(processId, { id, type, ...fields });
  return id;
}
function rpc(type, fields = {}, timeout) {
  const id = send(type, fields);
  const response = receiveUntil(
    processId,
    (m) => m.type === "response" && m.id === id,
    timeout,
  );
  if (!response.success)
    failure("invalid_state", response.error || "Pi command failed");
  return response.data;
}
// Tool search and exposure were released in Pi 0.99.0.
function supportsToolSearch(version) {
  const match = /(?:^|[^0-9])(\d+)\.(\d+)\.(\d+)(?=$|[^0-9A-Za-z.+-])/.exec(version || "");
  return !!match && (Number(match[1]) > 0 || Number(match[2]) >= 99);
}
function extension(tools, nativeSearch) {
  return "const tools=" + JSON.stringify(tools) + ";const nativeSearch=" + nativeSearch + ";const install=(" + installTools.toString() + ")(tools,nativeSearch);export default install;";
}
function installTools(tools, nativeSearch) {
  const fixed = new Set(["message_send", "attention_set", "chat_messages"]);
  const deferred = tools.filter(t => !fixed.has(t.name));
  async function invoke(tool, toolCallId, params, signal, context) {
    const raw = await context.ui.input("meerkat-tool-result", JSON.stringify({ tool_call_id: toolCallId, tool_name: tool.name, arguments: params }), { signal });
    if (raw === undefined) throw Error("Tool cancelled");
    const result = JSON.parse(raw);
    const text = [result.content_markdown, ...(result.resources || []).map(r => r.file_name + ": " + r.local_path)].filter(Boolean).join("\n\n") || JSON.stringify(result.structured_content);
    if (!result.success) throw Error(result.error?.message || text || "Tool failed");
    return { content: [{ type: "text", text }], details: result.structured_content };
  }
  // The generated extension is loaded as an ES module by the CLI.
  return function(pi) {
    for (const tool of tools.filter(t => nativeSearch || fixed.has(t.name))) {
      pi.registerTool({ name: tool.nativeName, label: tool.name, description: tool.description, parameters: tool.input_schema,
        ...(nativeSearch ? { exposure: fixed.has(tool.name) ? "direct" : "deferred" } : {}),
        execute: (id, params, signal, _update, context) => invoke(tool, id, params, signal, context) });
    }
    if (nativeSearch) {
      pi.on("session_start", () => pi.setActiveTools([...new Set([...pi.getActiveTools(), "tool_search"])]));
    } else if (deferred.length) {
      pi.registerTool({ name: "meerkat_tools", label: "Meerkat tools",
        description: "Discover and call additional Meerkat tools: reactions, tasks, profiles, message editing and recall. argv: [list], [describe, TOOL], or [call, TOOL, JSON_ARGUMENTS].",
        parameters: { type: "object", properties: { argv: { type: "array", items: { type: "string" }, minItems: 1 } }, required: ["argv"], additionalProperties: false },
        async execute(id, { argv }, signal, _update, context) {
          let value;
          if (argv.length === 1 && argv[0] === "list") value = deferred.map(t => ({ name: t.name, description: t.description }));
          else {
            const tool = deferred.find(t => t.name === argv[1]);
            if (!tool) throw Error("Unknown deferred tool; use [list]");
            if (argv[0] === "describe" && argv.length === 2) value = { name: tool.name, description: tool.description, input_schema: tool.input_schema };
            else if (argv[0] === "call" && argv.length === 3) return invoke(tool, id, JSON.parse(argv[2]), signal, context);
            else throw Error("Use [list], [describe, TOOL], or [call, TOOL, JSON_ARGUMENTS]");
          }
          return { content: [{ type: "text", text: JSON.stringify(value) }], details: value };
        }
      });
    }
  };
}

function open(value, resume) {
  request = value;
  paths = prepare(request);
  ctx.skills(request.working_directory + "/skills");
  const sessionDir = paths.root + "/sessions"; // Creating a marker keeps the session directory Machine-owned.
  ctx.write(sessionDir + "/.meerkat", "");
  if (resume) {
    const matches = ctx
      .list(sessionDir)
      .filter((p) => p.endsWith("_" + request.native_session_id + ".jsonl"));
    if (matches.length !== 1)
      failure("session_not_found", "Pi session artifact is unavailable for resume");
    const header = JSON.parse(ctx.readFirstLine(matches[0]));
    if (
      header.type !== "session" ||
      header.id !== request.native_session_id ||
      (header.cwd &&
        header.cwd !== request.working_directory &&
        header.cwd !== ctx.canonical(request.working_directory))
    )
      failure("bind_failed", "Pi session header mismatch");
  }
  const tools = request.tools.map((t, i) => {
    const nativeName =
      "meerkat_" + i + "_" + t.name.replace(/[^a-zA-Z0-9_-]/g, "_");
    toolNames.set(nativeName, t.name);
    return { ...t, nativeName };
  });
  const extensionPath = paths.root + "/extension-" + ctx.uuid() + ".ts";
  const version = request.cli_version;
  ctx.write(extensionPath, extension(tools, supportsToolSearch(version)));
  const args = [
    "--mode",
    "rpc",
    "--model",
    request.config.model.model_id,
    "--extension",
    extensionPath,
    "--no-themes",
    "--session-dir",
    sessionDir,
    "--skill",
    request.working_directory + "/skills",
    "--system-prompt",
    request.system_prompt,
    "--approve",
  ];
  if (request.config.thinking_level)
    args.push("--thinking", request.config.thinking_level);
  if (resume) args.push("--session-id", request.native_session_id);
  const recordRoot =
    request.config.environment.MEERKAT_PI_ARTIFACT_DIR ||
    paths.root + "/records";
  const recordPath =
    request.config.environment.MEERKAT_PI_RECORD_PATH ||
    recordRoot + "/pi-" + ctx.uuid() + ".rpc.jsonl";
  processId = launch(request.config, args, {
    cwd: request.working_directory,
    env: { ...paths.env, PI_CODING_AGENT_SESSION_DIR: sessionDir },
    stderr: recordRoot + "/pi-" + ctx.uuid() + ".stderr.log",
    record: {
      path: recordPath,
      limit:
        Number(request.config.environment.MEERKAT_PI_RECORD_MAX_BYTES) ||
        4 * 1024 * 1024,
    },
  });
  // Pi answers only after installing its packages, which a fresh home lacks.
  const state = rpc("get_state", {}, 180000);
  nativeId = state?.sessionId;
  if (!nativeId) failure("bind_failed", "Pi get_state omitted sessionId");
  ctx.bindSession(nativeId);
  return { native_session_id: nativeId };
}
function assistant(message) {
  if (message?.role !== "assistant") return null;
  return (message.content || [])
    .filter((p) => p.type === "text")
    .map((p) => p.text || "")
    .join("");
}
function inject(id, value) {
  ctx.send(processId, {
    type: "extension_ui_response",
    id,
    value: JSON.stringify(value),
  });
}
defineHarness({
  reuseSession: true,
  apiVersion: 1,
  id: "pi",
  displayName: "Pi",
  defaults: { environment: {}, launch_arguments: [] },
  probe: probeVersion,
  models(config) {
    const isolated = prepare({ working_directory: ctx.directory(), config });
    processId = launch(
      config,
      ["--mode", "rpc", "--no-session", "--no-extensions", "--no-themes"],
      { cwd: ctx.directory(), env: isolated.env },
    );
    try {
      const result = rpc("get_available_models");
      if (!Array.isArray(result?.models))
        failure("invalid_event", "Pi did not return a model directory");
      return result.models.map((m) => ({
        id: m.provider + "/" + m.id,
        name: m.name || m.id,
        description: "",
        thinking_levels: m.reasoning
          ? ["off", "minimal", "low", "medium", "high", "xhigh"]
          : ["off"],
      }));
    } finally {
      ctx.terminate(processId);
    }
  },
  test(request) {
    return {
      request,
      input: "Reply with one short greeting. Do not use tools.",
    };
  },
  start: (value) => open(value, false),
  resume: (value) => open(value, true),
  rebind(value) {
    request = value;
    const env = request.config.environment;
    if (env.MEERKAT_PI_RECORD_PATH)
      ctx.record(processId, { path: env.MEERKAT_PI_RECORD_PATH,
        limit: Number(env.MEERKAT_PI_RECORD_MAX_BYTES) || 4 * 1024 * 1024 });
  },
  execute(value) {
    active = true;
    output = "";
    cancelReason = null;
    observed.clear();
    uiRequests.clear();
    results.clear();
    contextTokens = 0;
    started(nativeId);
    rpc("prompt", { message: value.input });
  },
  steer: (value) => rpc("steer", { message: value.input }),
  toolResult(value) {
    if (!observed.has(value.tool_call_id))
      failure("invalid_state", "Pi tool call was not observed");
    results.set(value.tool_call_id, value);
    if (uiRequests.has(value.tool_call_id))
      inject(uiRequests.get(value.tool_call_id), value);
  },
  cancel(value) {
    if (compaction) {
      // Pi cannot abort a compaction; the turn itself already finished.
      active = false;
      const text = compaction.text;
      compaction = { id: compaction.id, text: null };
      completed(text);
      return { accepted: true };
    }
    cancelReason = value.reason;
    cancelAt = Date.now();
    send("abort");
    ctx.cancelAfter(value.reason, 3000);
    return { accepted: true };
  },
  close() {
    if (active) {
      active = false;
      ctx.emit({ kind: "cancelled", reason: "Session closed" });
    }
    if (processId) ctx.terminate(processId);
  },
  onExit(packet) {
    if (packet.process === processId && !active)
      failure("stream_interrupted", packet.detail || "Pi process exited");
    if (packet.process === processId && active) {
      active = false;
      failed(packet.detail || "Pi process exited", "stream_interrupted");
    }
  },
  onMessage(packet) {
    if (packet.process !== processId) return;
    const v = packet.message;
    if (v.invalid_output) failure("invalid_event", "Invalid Pi RPC event");
    if (v.type === "response") {
      if (compaction && v.id === compaction.id) {
        // The turn already finished; a failed compaction leaves Pi's own.
        const text = compaction.text;
        compaction = null;
        if (text !== null) {
          active = false;
          completed(text);
        }
        return;
      }
      if (v.command === "prompt" && !v.success && active) {
        active = false;
        failed(v.error || "Pi rejected prompt");
      }
      return;
    }
    if (!active) return;
    switch (v.type) {
      case "extension_ui_request":
        if (v.method === "input" && v.title === "meerkat-tool-result") {
          const invocation = JSON.parse(v.placeholder);
          const call = invocation.tool_call_id;
          if (!observed.has(call)) {
            const tool = request.tools.find(t => t.name === invocation.tool_name);
            if (!tool || ["message_send", "attention_set", "chat_messages"].includes(tool.name))
              failure("invalid_event", "Unknown Pi deferred tool");
            observed.add(call);
            ctx.emit({ kind: "tool_started", tool_call_id: call, tool_name: tool.name, arguments: invocation.arguments || {} });
          }
          uiRequests.set(call, v.id);
          if (results.has(call)) inject(v.id, results.get(call));
        }
        break;
      case "message_update": {
        const e = v.assistantMessageEvent;
        if (e?.type === "text_delta") {
          output += e.delta || "";
          ctx.emit({ kind: "assistant_delta", text: e.delta || "" });
        } else if (e?.type === "thinking_delta")
          ctx.emit({ kind: "reasoning_delta", text: e.delta || "" });
        break;
      }
      case "message_end": {
        const text = assistant(v.message);
        if (text !== null) output = text;
        if (v.message?.usage) {
          contextTokens =
            (v.message.usage.input || 0) +
            (v.message.usage.cacheRead || 0) +
            (v.message.usage.cacheWrite || 0);
          usage(contextTokens, v.message.usage.output);
        }
        break;
      }
      case "tool_execution_start":
        if (toolNames.has(v.toolName)) {
          observed.add(v.toolCallId);
          ctx.emit({
            kind: "tool_started",
            tool_call_id: v.toolCallId,
            tool_name: toolNames.get(v.toolName),
            arguments: v.args || {},
          });
        }
        break;
      case "tool_execution_end":
        if (observed.has(v.toolCallId))
          ctx.emit({
            kind: "tool_finished",
            tool_call_id: v.toolCallId,
            success: !v.isError,
            output: results.get(v.toolCallId) || v.result || null,
          });
        break;
      case "agent_end": {
        const last = (v.messages || [])
          .filter((m) => m.role === "assistant")
          .pop();
        const limit = compactTokens(request.config);
        if (!cancelReason && !["error", "aborted"].includes(last?.stopReason) && limit && contextTokens > limit) {
          compaction = { id: send("compact"), text: assistant(last) ?? output };
          break;
        }
        active = false;
        if (cancelReason) ctx.emit({ kind: "cancelled", reason: cancelReason });
        else if (["error", "aborted"].includes(last?.stopReason))
          failed(last.errorMessage || last.stopReason);
        else completed(assistant(last) ?? output);
        break;
      }
    }
  },
});
