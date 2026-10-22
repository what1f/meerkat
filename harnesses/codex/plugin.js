"use strict";
function prepare(request) {
  ctx.skills(request.working_directory + "/.agents/skills");
  return prepareNative(request, {
    sourceVariable: "CODEX_HOME",
    sourceRelative: ".codex",
    files: ["auth.json", "config.toml", "config.json", "instructions.md"],
    resources: ["skills", "plugins", "AGENTS.md"],
    pathKeys: [
      "model_instructions_file",
      "experimental_instructions_file",
      "cwd",
    ],
    directoryVariables: ["CODEX_SQLITE_HOME"],
  });
}
let request,
  paths,
  processId,
  nativeId,
  turnId,
  active = false,
  output = "",
  nextId = 1,
  cancelReason = null;
const calls = new Map();
function rpc(method, params) {
  const id = nextId++;
  ctx.send(processId, { id, method, params });
  const response = receiveUntil(processId, (m) => m.id === id && !m.method);
  if (response.error)
    failure(
      method === "thread/resume" && /(?:thread|session).*(?:not found|does not exist)|no rollout found/i.test(response.error.message || "") ? "session_not_found" : "invalid_event",
      response.error.message || JSON.stringify(response.error),
    );
  return response.result;
}
function open(value, resume) {
  request = value;
  paths = prepare(request);
  processId = launch(request.config, ["app-server", "--listen", "stdio://"], {
    cwd: request.working_directory,
    env: paths.env,
    stderr: paths.root + "/app-server-" + ctx.uuid() + ".stderr.log",
  });
  rpc("initialize", {
    clientInfo: { name: "lankide", version: "1" },
    capabilities: { experimentalApi: true },
  });
  ctx.send(processId, { method: "initialized", params: {} });
  const params = {
    model: request.config.model.model_id,
    cwd: request.working_directory,
    approvalPolicy: "never",
    sandbox: "danger-full-access",
    developerInstructions: request.system_prompt,
    dynamicTools: request.tools.map((t) => ({
      type: "function",
      name: t.name,
      description: t.description,
      inputSchema: t.input_schema,
      deferLoading: !["message_send", "attention_set", "chat_messages"].includes(t.name),
    })),
  };
  if (resume) {
    params.threadId = request.native_session_id;
    params.excludeTurns = true;
  }
  const result = rpc(resume ? "thread/resume" : "thread/start", params);
  nativeId = result.thread?.id;
  if (!nativeId) failure("bind_failed", "Codex returned no thread ID");
  ctx.bindSession(nativeId);
  return { native_session_id: nativeId };
}
defineHarness({
  apiVersion: 1,
  id: "codex",
  displayName: "Codex",
  defaults: { environment: {}, launch_arguments: [] },
  probe: probeVersion,
  models(config) {
    const isolated = prepare({ working_directory: ctx.directory(), config });
    processId = launch(config, ["app-server", "--listen", "stdio://"], {
      cwd: ctx.directory(),
      env: isolated.env,
    });
    try {
      rpc("initialize", {
        clientInfo: { name: "lankide-catalog", version: "1" },
        capabilities: { experimentalApi: true },
      });
      ctx.send(processId, { method: "initialized", params: {} });
      const models = [],
        seen = new Set();
      let cursor = null;
      do {
        const page = rpc("model/list", { limit: 100, cursor });
        if (!Array.isArray(page.data))
          failure("invalid_event", "Codex did not return a model directory");
        for (const m of page.data) {
          if (!m.hidden)
            models.push({
              id: m.model,
              name: m.displayName || m.model,
              description: m.description || "",
              thinking_levels: (m.supportedReasoningEfforts || []).map(
                (e) => e.reasoningEffort,
              ),
            });
        }
        cursor = page.nextCursor;
        if (cursor && seen.has(cursor))
          failure("invalid_event", "Repeated model directory cursor");
        seen.add(cursor);
        if (seen.size > 100)
          failure("invalid_event", "Model directory exceeded pagination limit");
      } while (cursor);
      return models;
    } finally {
      ctx.terminate(processId);
    }
  },
  test(request) {
    return {
      request: {
        ...request,
        config: {
          ...request.config,
          model: { ...request.config.model, model_id: "gpt-6-luna" },
        },
      },
      input: "Reply with one short greeting. Do not use tools.",
    };
  },
  start: (value) => open(value, false),
  resume: (value) => open(value, true),
  execute(value) {
    active = true;
    output = "";
    turnId = null;
    cancelReason = null;
    started(nativeId);
    const result = rpc("turn/start", {
      threadId: nativeId,
      input: [{ type: "text", text: value.input, text_elements: [] }],
      effort: request.config.thinking_level || null,
    });
    turnId = result.turn?.id || turnId;
  },
  steer(value) {
    if (!active || !turnId)
      failure("invalid_state", "Codex has no active turn");
    rpc("turn/steer", {
      threadId: nativeId,
      expectedTurnId: turnId,
      input: [{ type: "text", text: value.input, text_elements: [] }],
    });
  },
  toolResult(value) {
    const id = calls.get(value.tool_call_id);
    if (id === undefined)
      failure("invalid_state", "Codex tool call was not observed");
    ctx.send(processId, {
      id,
      result: {
        contentItems: [
          {
            type: "inputText",
            text: JSON.stringify({
              content_markdown: value.content_markdown,
              structured_content: value.structured_content,
              resources: value.resources,
              error: value.error,
            }),
          },
        ],
        success: value.success,
      },
    });
    calls.delete(value.tool_call_id);
  },
  cancel(value) {
    cancelReason = value.reason;
    if (active && turnId) rpc("turn/interrupt", { threadId: nativeId, turnId });
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
    if (packet.process === processId && active) {
      active = false;
      failed(packet.detail || "Codex process exited", "stream_interrupted");
    }
  },
  onMessage(packet) {
    if (packet.process !== processId) return;
    const m = packet.message;
    if (m.invalid_output) {
      if (active) failure("invalid_event", "Invalid Codex JSON output");
      return;
    }
    const p = m.params || {};
    if (m.id !== undefined && m.method) {
      if (m.method === "item/tool/call") {
        calls.set(p.callId, m.id);
        ctx.emit({
          kind: "tool_started",
          tool_call_id: p.callId,
          tool_name: p.tool,
          arguments: p.arguments ?? {},
        });
      } else
        ctx.send(processId, {
          id: m.id,
          error: { code: -32601, message: "Unsupported request" },
        });
      return;
    }
    if (!active || (p.threadId && p.threadId !== nativeId)) return;
    switch (m.method) {
      case "turn/started":
        turnId = p.turn?.id;
        break;
      case "item/agentMessage/delta":
        output += p.delta || "";
        ctx.emit({ kind: "assistant_delta", text: p.delta || "" });
        break;
      case "item/reasoning/summaryTextDelta":
      case "item/reasoning/textDelta":
        ctx.emit({ kind: "reasoning_delta", text: p.delta || "" });
        break;
      case "thread/tokenUsage/updated":
        usage(
          p.tokenUsage?.last?.inputTokens,
          p.tokenUsage?.last?.outputTokens,
        );
        break;
      case "item/completed":
        if (p.item?.type === "agentMessage") {
          if (!output && p.item.text)
            ctx.emit({ kind: "assistant_delta", text: p.item.text });
          output = p.item.text || output;
        }
        if (p.item?.type === "dynamicToolCall")
          ctx.emit({
            kind: "tool_finished",
            tool_call_id: p.item.id,
            success: p.item.success === true,
            output: p.item.contentItems || [],
          });
        break;
      case "warning":
      case "configWarning":
      case "deprecationNotice":
        ctx.emit({ kind: "warning", message: p.message || "Codex warning" });
        break;
      case "turn/completed":
        if (turnId && p.turn?.id && turnId !== p.turn.id) return;
        active = false;
        if (p.turn?.status === "completed") completed(output);
        else if (p.turn?.status === "interrupted")
          ctx.emit({
            kind: "cancelled",
            reason: cancelReason || "Turn interrupted",
          });
        else failed(p.turn?.error?.message || "Codex turn failed");
        break;
    }
  },
});
