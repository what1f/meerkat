"use strict";
function prepare(request) {
  ctx.skills(request.working_directory + "/.claude/skills");
  return prepareNative(request, {
    sourceVariable: "CLAUDE_CONFIG_DIR",
    sourceRelative: ".claude",
    files: ["settings.json", ".credentials.json", ".claude.json"],
    resources: ["skills", "commands", "agents", "plugins", "CLAUDE.md"],
  });
}
let request,
  paths,
  processId,
  nativeId,
  mcpPath,
  active = false,
  output = "",
  contextTokens = 0,
  terminal = false;
function sessionExists() {
  const dirs = [paths.directory + "/projects"];
  while (dirs.length) {
    let entries;
    try {
      entries = ctx.list(dirs.pop());
    } catch {
      continue;
    }
    for (const file of entries) {
      if (file.endsWith("/" + nativeId + ".jsonl")) return true;
      try {
        ctx.list(file);
        dirs.push(file);
      } catch {}
    }
  }
  return false;
}
function open(value, resume) {
  request = value;
  paths = prepare(request);
  nativeId = resume ? request.native_session_id : ctx.uuid();
  if (!/^[0-9a-f-]{36}$/i.test(nativeId))
    failure("invalid_config", "Claude native session ID must be a UUID");
  if (resume && !sessionExists())
    failure("session_not_found", "Claude session artifact is unavailable for resume");
  mcpPath = ctx.mcp(paths.root + "/runs/" + ctx.uuid(), request.tools.map(t => ({
    ...t, _meta: { "anthropic/alwaysLoad": ["message_send", "attention_set", "chat_messages"].includes(t.name) }
  })));
  // Wait for the local server before the first prompt. Per-tool false metadata
  // keeps the remaining tools deferred even with this server-level default.
  const mcpConfig = JSON.parse(ctx.read(mcpPath));
  mcpConfig.mcpServers.meerkat.alwaysLoad = true;
  ctx.write(mcpPath, JSON.stringify(mcpConfig));
  ctx.bindSession(nativeId);
  return { native_session_id: nativeId };
}
function input(text) {
  ctx.send(processId, {
    type: "user",
    message: { role: "user", content: [{ type: "text", text }] },
  });
}
defineHarness({
  reuseSession: true,
  apiVersion: 1,
  id: "claude_code",
  displayName: "Claude Code",
  defaults: { environment: { ENABLE_TOOL_SEARCH: "true" }, launch_arguments: [] },
  probe: probeVersion,
  // Claude Code takes these aliases and maps them to its current models.
  // Asking the CLI itself would start a session, so the list stays fixed.
  models() {
    const efforts = ["low", "medium", "high", "xhigh", "max"];
    return [
      ["default", "Default"],
      ["opus", "Opus"],
      ["fable", "Fable"],
      ["sonnet", "Sonnet"],
      ["haiku", "Haiku"],
    ].map(([id, name]) => ({ id, name, description: "", thinking_levels: efforts }));
  },
  test(request) {
    return {
      request: {
        ...request,
        config: {
          ...request.config,
          model: { ...request.config.model, model_id: "haiku" },
        },
      },
      input: "Reply with one short greeting. Do not use tools.",
    };
  },
  start: (value) => open(value, false),
  resume: (value) => open(value, true),
  rebind(value) { request = value; },
  execute(value) {
    active = true;
    terminal = false;
    output = "";
    contextTokens = 0;
    started(nativeId);
    if (processId) {
      input(value.input);
      return;
    }
    const args = [
      "-p",
      "--output-format",
      "stream-json",
      "--verbose",
      "--include-partial-messages",
      "--input-format",
      "stream-json",
      "--model",
      request.config.model.model_id,
      "--permission-mode",
      "bypassPermissions",
      "--mcp-config",
      mcpPath,
      "--append-system-prompt",
      request.system_prompt,
    ];
    const env = { ...paths.env, ENABLE_TOOL_SEARCH: request.config.environment.ENABLE_TOOL_SEARCH ?? "true" };
    // memory/ in the Agent home is the only long-term store, so it moves with the Agent.
    env.CLAUDE_CODE_DISABLE_AUTO_MEMORY = request.config.environment.CLAUDE_CODE_DISABLE_AUTO_MEMORY ?? "1";
    if (request.config.thinking_level === "off")
      env.CLAUDE_CODE_DISABLE_THINKING = "1";
    else if (request.config.thinking_level) {
      args.push("--effort", request.config.thinking_level);
      env.CLAUDE_CODE_DISABLE_THINKING = "0";
    }
    const compact = compactTokens(request.config);
    if (compact && request.config.environment.CLAUDE_CODE_AUTO_COMPACT_WINDOW === undefined)
      env.CLAUDE_CODE_AUTO_COMPACT_WINDOW = String(compact);
    // Sourced after the user's shell setup, which may rewrite PATH.
    if (request.config.environment.MEERKAT_PATH) {
      env.CLAUDE_ENV_FILE = paths.root + "/env.sh";
      ctx.write(env.CLAUDE_ENV_FILE, 'export PATH="$MEERKAT_PATH:$PATH"\n');
    }
    args.push(sessionExists() ? "--resume" : "--session-id", nativeId);
    processId = launch(request.config, args, {
      cwd: request.working_directory,
      env,
      stderr: paths.root + "/claude-" + ctx.uuid() + ".stderr.log",
    });
    input(value.input);
  },
  steer: (value) => input(value.input),
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
    if (active) {
      active = false;
      ctx.emit({ kind: "cancelled", reason: value.reason });
    }
    if (processId) ctx.terminate(processId);
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
      failure("stream_interrupted", packet.detail || "Claude process exited");
    if (packet.process === processId && active && !terminal) {
      active = false;
      failed(packet.detail || "Claude process exited", "stream_interrupted");
    }
  },
  onMessage(packet) {
    if (packet.process !== processId) return;
    const v = packet.message;
    if (!active) return;
    if (v.invalid_output)
      failure("invalid_event", "Invalid Claude stream-json event");
    if (
      v.type === "system" &&
      v.subtype === "init" &&
      v.session_id !== nativeId
    )
      failure("bind_failed", "Claude bound a different session");
    // Context size is the last model call's whole prompt, cached part included.
    const callUsage = v.type === "assistant" ? v.message?.usage : null;
    if (callUsage)
      contextTokens =
        (callUsage.input_tokens || 0) +
        (callUsage.cache_read_input_tokens || 0) +
        (callUsage.cache_creation_input_tokens || 0);
    if (v.type === "stream_event") {
      const delta = v.event?.delta;
      if (delta?.type === "text_delta") {
        output += delta.text || "";
        ctx.emit({ kind: "assistant_delta", text: delta.text || "" });
      } else if (delta?.type === "thinking_delta")
        ctx.emit({ kind: "reasoning_delta", text: delta.thinking || "" });
    }
    if (v.type === "result") {
      active = false;
      terminal = true;
      if (v.session_id !== nativeId)
        failed("Claude result used a different session", "bind_failed");
      else if (v.is_error)
        failed(
          (v.errors || []).join("\n") ||
            v.error?.message ||
            v.error ||
            v.result ||
            v.subtype ||
            "Claude execution failed",
        );
      else {
        usage(contextTokens, v.usage?.output_tokens);
        completed(v.result || output);
      }
    }
  },
});
