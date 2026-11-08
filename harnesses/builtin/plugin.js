"use strict";
// Built-in Agent over ACP (newline-delimited JSON-RPC 2.0 on stdio).
// The Rust wrapper adds the shared home and credentials to the environment.
let request,
  processId,
  nativeId,
  active = false,
  nextId = 1,
  promptId = null,
  cancelReason = null,
  finished = null,
  messageId = null,
  lastText = "",
  output = "";
// Steering prompts in flight, by request id; a dropped one is sent again
// as the next prompt of the same execution.
const steers = new Map(),
  queued = [];

// Keep native product names out of anything a user can read.
function neutral(text) {
  return String(text || "").replace(/(?:𝒇x|\bfx\b)/gi, "Built-in Agent");
}
function send(method, params) {
  const id = nextId++;
  ctx.send(processId, { jsonrpc: "2.0", id, method, params });
  return id;
}
function notify(method, params) {
  ctx.send(processId, { jsonrpc: "2.0", method, params });
}
function rpc(method, params, timeout = 30000) {
  const id = send(method, params);
  const response = receiveUntil(
    processId,
    (m) => m.id === id && !m.method,
    timeout,
  );
  if (response.error) {
    const message = response.error.message || JSON.stringify(response.error);
    failure(
      method === "session/resume" && /session not found/i.test(message)
        ? "session_not_found"
        : "invalid_event",
      neutral(message),
    );
  }
  return response.result;
}
function initialize() {
  rpc("initialize", {
    protocolVersion: 1,
    clientCapabilities: {
      fs: { readTextFile: false, writeTextFile: false },
      terminal: false,
      // Diagnostics arrive as notices instead of assistant text.
      session: { notices: {} },
    },
    clientInfo: { name: "meerkat", version: "1" },
  });
}
function mcpServers(path) {
  const server = JSON.parse(ctx.read(path)).mcpServers.meerkat;
  return [
    {
      type: "http",
      name: "meerkat",
      url: server.url,
      headers: Object.entries(server.headers || {}).map(([name, value]) => ({
        name,
        value,
      })),
    },
  ];
}
function open(value, resume) {
  request = value;
  const directory = request.working_directory;
  const root = directory + "/.meerkat/builtin";
  ctx.skills(directory + "/skills");
  // MCP requires `_meta` to be an object when present.
  const mcpPath = ctx.mcp(
    root + "/runs/" + ctx.uuid(),
    request.tools.map((t) => ({ ...t, _meta: {} })),
  );
  // Full access is a process-only flag, so it is never saved to settings.
  const args = ["acp", "--full-access"];
  if (request.config.model.model_id)
    args.push("--model", request.config.model.model_id);
  // The level list offers "disable"; the agent spells it "none".
  const effort = request.config.thinking_level;
  if (effort) args.push("--effort", effort === "disable" ? "none" : effort);
  processId = launch(request.config, args, {
    cwd: directory,
    stderr: root + "/agent-" + ctx.uuid() + ".stderr.log",
  });
  initialize();
  const params = { cwd: directory, mcpServers: mcpServers(mcpPath) };
  if (resume) {
    rpc("session/resume", { ...params, sessionId: request.native_session_id });
    nativeId = request.native_session_id;
  } else {
    if (request.system_prompt.trim())
      params.systemPrompt = [{ type: "text", text: request.system_prompt }];
    nativeId = rpc("session/new", params)?.sessionId;
  }
  if (!nativeId) failure("bind_failed", "No session was returned");
  ctx.bindSession(nativeId);
  return { native_session_id: nativeId };
}
function prompt(text, steer) {
  const params = { sessionId: nativeId, prompt: [{ type: "text", text }] };
  if (steer) params._meta = { fx: { steer: true } };
  return send("session/prompt", params);
}
function fail(message, code = "agent_error") {
  active = false;
  const error = classifyError({ message: neutral(message), code });
  ctx.emit({
    kind: "failed",
    code,
    message: neutral(message),
    failure: error.failure,
  });
}
// The turn ends once the prompt has stopped and every steer has answered.
function settle() {
  if (!active || !finished || steers.size) return;
  if (queued.length && !cancelReason) {
    finished = null;
    promptId = prompt(queued.splice(0).join("\n\n"), false);
    return;
  }
  active = false;
  completed(lastText || output);
}
function models(config) {
  const process = launch(config, ["models", "--json"], { cwd: ctx.directory() });
  let directory = null;
  try {
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline && !directory) {
      const packet = ctx.receive(100);
      if (!packet || packet.process !== process) continue;
      if (packet.exit) {
        if (packet.success === false)
          failure("invalid_config", neutral(packet.detail || "Model list failed"));
        break;
      }
      if (packet.message?.kind === "models") directory = packet.message;
    }
  } finally {
    ctx.terminate(process);
  }
  // An unreachable model service is reported in-band with a zero exit.
  if (directory?.error && !Array.isArray(directory.models))
    failure("stream_interrupted", "Could not reach the model service");
  if (!Array.isArray(directory?.models))
    failure("invalid_event", "No model directory was returned");
  return directory.models.map((m) => ({
    id: m.id,
    name: m.name || m.id,
    description: m.description || "",
    thinking_levels: m.efforts,
  }));
}
// Effort choices the current connection offers, read from a session that is
// never prompted and therefore never saved.
function efforts(config) {
  processId = launch(config, ["acp"], { cwd: ctx.directory() });
  try {
    initialize();
    const session = rpc("session/new", { cwd: ctx.directory(), mcpServers: [] });
    const option = (session?.configOptions || []).find((o) => o.id === "effort");
    return (option?.options || [])
      .map((o) => o.value)
      .filter((v) => typeof v === "string" && v !== "auto");
  } finally {
    ctx.terminate(processId);
  }
}
defineHarness({
  reuseSession: true,
  apiVersion: 1,
  id: "builtin",
  displayName: "Built-in Agent",
  defaults: { environment: {}, launch_arguments: [] },
  probe(config) {
    const result = probeVersion(config);
    const match = /\d+\.\d+\.\d+[0-9A-Za-z.+-]*/.exec(result.version || "");
    result.version = match ? match[0] : null;
    if (result.availability !== "available")
      result.reason = "Built-in Agent could not start";
    return result;
  },
  models(config) {
    const list = models(config);
    if (list.some((m) => !Array.isArray(m.thinking_levels))) {
      let fallback = [];
      try {
        fallback = efforts(config);
      } catch (_) {}
      for (const m of list)
        if (!Array.isArray(m.thinking_levels)) m.thinking_levels = fallback;
    }
    for (const m of list)
      m.thinking_levels = m.thinking_levels.filter((v) => v !== "auto");
    return list;
  },
  start: (value) => open(value, false),
  resume: (value) => open(value, true),
  rebind(value) {
    request = value;
  },
  execute(value) {
    active = true;
    output = "";
    lastText = "";
    messageId = null;
    cancelReason = null;
    finished = null;
    steers.clear();
    queued.length = 0;
    started(nativeId);
    promptId = prompt(value.input, false);
  },
  steer(value) {
    if (!active) failure("invalid_state", "No turn is running");
    if (finished) queued.push(value.input);
    else steers.set(prompt(value.input, true), value.input);
  },
  toolResult(value) {
    ctx.mcpResult(value);
    ctx.emit({
      kind: "tool_finished",
      tool_call_id: value.tool_call_id,
      success: value.success,
      output: value,
    });
  },
  onToolCall(value) {
    ctx.emit({
      kind: "tool_started",
      tool_call_id: value.tool_call_id,
      tool_name: value.tool_name,
      arguments: value.arguments,
    });
  },
  cancel(value) {
    cancelReason = value.reason;
    if (active) notify("session/cancel", { sessionId: nativeId });
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
    if (packet.process !== processId) return;
    if (!active)
      failure("stream_interrupted", neutral(packet.detail || "Agent process exited"));
    fail(packet.detail || "Agent process exited", "stream_interrupted");
  },
  onMessage(packet) {
    if (packet.process !== processId) return;
    const m = packet.message;
    if (m.invalid_output) {
      if (active) failure("invalid_event", "Invalid Agent output");
      return;
    }
    if (m.method && m.id !== undefined) {
      // Full access needs no permission prompts; refuse anything else.
      ctx.send(processId, {
        jsonrpc: "2.0",
        id: m.id,
        error: { code: -32601, message: "Unsupported request" },
      });
      return;
    }
    if (m.id !== undefined && !m.method) {
      if (steers.has(m.id)) {
        const text = steers.get(m.id);
        steers.delete(m.id);
        if (m.error || m.result?._meta?.fx?.steering === "dropped") queued.push(text);
        settle();
        return;
      }
      if (m.id !== promptId || !active) return;
      if (m.error) {
        if (cancelReason) {
          active = false;
          ctx.emit({ kind: "cancelled", reason: cancelReason });
        } else fail(m.error.message || "Agent request failed");
        return;
      }
      const result = m.result || {};
      if (result.usage)
        usage(result.usage.inputTokens || 0, result.usage.outputTokens);
      if (result.stopReason === "cancelled") {
        active = false;
        ctx.emit({ kind: "cancelled", reason: cancelReason || "Turn cancelled" });
      } else if (result.stopReason === "refusal") fail("The model refused the request");
      else {
        finished = result;
        settle();
      }
      return;
    }
    if (m.method !== "session/update" || !active) return;
    const p = m.params || {};
    if (p.sessionId && p.sessionId !== nativeId) return;
    const update = p.update || {};
    switch (update.sessionUpdate) {
      case "agent_message_chunk": {
        const text = update.content?.type === "text" ? update.content.text || "" : "";
        if (update.messageId !== messageId) {
          messageId = update.messageId;
          lastText = "";
        }
        lastText += text;
        output += text;
        if (text) ctx.emit({ kind: "assistant_delta", text });
        break;
      }
      case "agent_thought_chunk": {
        const text = update.content?.type === "text" ? update.content.text || "" : "";
        if (text) ctx.emit({ kind: "reasoning_delta", text });
        break;
      }
      case "user_message_chunk": {
        // A steer was taken into the running turn.
        const id = update._meta?.fx?.steering?.requestId;
        if (id !== undefined && steers.has(id)) {
          steers.delete(id);
          settle();
        }
        break;
      }
    }
  },
});
