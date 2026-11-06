"use strict";
let harness;
const capabilities = [
  "system_prompt",
  "streaming",
  "resume",
  "steer",
  "cancel",
  "tools",
];
function defineHarness(plugin) {
  harness = plugin;
}
function host(op, value = null) {
  const r = JSON.parse(__host(op, JSON.stringify(value)));
  if (r.error) throw r.error;
  return r.ok;
}
function failure(code, message) {
  const e = new Error(message);
  e.code = code;
  throw e;
}
function classifyError(error) {
  const message = error.message || String(error),
    text = message.toLowerCase();
  let category = "agent";
  if (
    /model.*(not found|not available|not supported)|unknown model|invalid model|does not exist|no such file|permission denied|enoent|command not found/.test(
      text,
    )
  )
    category = "configuration";
  else if (
    /unauthor|authenticat|invalid.*(key|token)|token expired|no api key|missing api key|not logged in|login required|401|forbidden|403/.test(
      text,
    )
  )
    category = "authentication";
  else if (
    /rate.?limit|usage.?limit|resource_exhausted|quota|too many requests|429/.test(
      text,
    )
  )
    category = "rate_limit";
  else if (/cancelled|canceled|aborted/.test(text)) category = "cancelled";
  else if (/timeout|timed out/.test(text)) category = "timeout";
  else if (
    /exited|exit status|process exit|stream|connection|503|502|temporarily unavailable/.test(
      text,
    )
  )
    category = "transient";
  if (
    category === "agent" &&
    ["spawn_failed", "stream_interrupted", "io"].includes(error.code)
  )
    category = "transient";
  if (error.failure && category === "agent") return error;
  const code = error.code || "invalid_event";
  return {
    code,
    message,
    failure: {
      code: "harness_" + code,
      category,
      retryable: ["rate_limit", "transient", "timeout"].includes(category),
      detail: message,
    },
  };
}
function __invoke(hook, input) {
  try {
    const result = harness[hook]?.(JSON.parse(input));
    return JSON.stringify({ ok: result ?? null });
  } catch (e) {
    return JSON.stringify({
      error: (harness.classifyError || classifyError)(e),
    });
  }
}
const ctx = {
  directory: () => host("directory"),
  fail: (error) => host("fail", error),
  cancelAfter: (reason, millis = 3000) =>
    host("cancelAfter", { reason, millis }),
  emit: (event) => host("emit", event),
  bindSession: (id) => host("bind", id),
  uuid: () => host("uuid"),
  env: (name) => host("env", name),
  write: (path, content) => host("write", { path, content }),
  canonical: (path) => host("canonical", path),
  read: (path) => host("read", path),
  readFirstLine: (path) => host("readFirstLine", path),
  exists: (path) => host("exists", path),
  list: (path) => host("list", path),
  prepare: (plan) => host("prepare", plan),
  skills: (path) => host("skills", path),
  mcp: (directory, tools) => host("mcp", { directory, tools }),
  mcpResult: (result) => host("mcpResult", result),
  spawn: (options) => host("spawn", options),
  send: (process, message) => host("send", { process, message }),
  record: (process, options) => host("record", { process, options }),
  terminate: (process) => host("terminate", process),
  closeStdin: (process) => host("closeStdin", process),
  receive: (timeout) => host("receive", { timeout }),
};
function dispatch(packet) {
  if (packet.exit) harness.onExit?.(packet);
  else if (packet.mcp) harness.onToolCall?.(packet);
  else harness.onMessage?.(packet);
}
function receiveUntil(process, matches, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const packet = ctx.receive(100);
    if (!packet) continue;
    if (packet.process === process && packet.exit)
      failure("stream_interrupted", packet.detail || "Native process exited");
    if (packet.process === process && matches(packet.message))
      return packet.message;
    dispatch(packet);
  }
  failure("timeout", "Native RPC timed out");
}
function launch(config, args, extra = {}) {
  return ctx.spawn({
    executable: config.cli_path,
    args: [...args, ...config.launch_arguments],
    ...extra,
    env: { ...config.environment, ...extra.env },
  });
}
function probeVersion(config) {
  let process;
  try {
    process = launch(config, ["--version"]);
    const deadline = Date.now() + 5000;
    let version = "",
      exited = false;
    while (Date.now() < deadline) {
      const packet = ctx.receive(100);
      if (!packet) continue;
      if (packet.process !== process) continue;
      if (packet.exit) {
        if (packet.success === false)
          failure(
            "invalid_config",
            packet.detail || "Native version probe failed",
          );
        exited = true;
        break;
      }
      const m = packet.message;
      version += m.invalid_output ?? JSON.stringify(m);
    }
    if (!exited) failure("timeout", "Native version probe timed out");
    if (!version.trim())
      failure("invalid_config", "Native version probe returned no version");
    ctx.terminate(process);
    return {
      info: { harness_id: harness.id, display_name: harness.displayName },
      availability: "available",
      version: version.trim() || null,
      capabilities,
      reason: null,
    };
  } catch (e) {
    if (process) ctx.terminate(process);
    return {
      info: { harness_id: harness.id, display_name: harness.displayName },
      availability: "unavailable",
      version: null,
      capabilities,
      reason: e.message,
    };
  }
}
function started(id) {
  ctx.emit({ kind: "started" });
  ctx.emit({ kind: "native_session_bound", native_session_id: id });
}
function completed(text) {
  ctx.emit({ kind: "completed", final_output: text });
}
function failed(message, code = "agent_error") {
  const error = classifyError({ message, code });
  ctx.emit({ kind: "failed", code, message, failure: error.failure });
}
/** Context size at which the native CLI compacts; 0 keeps the CLI default. */
function compactTokens(config) {
  const value = config.environment.MEERKAT_COMPACT_TOKENS;
  if (value === undefined || value.trim() === "") return 128000;
  const tokens = Math.floor(Number(value));
  return tokens > 0 ? tokens : 0;
}
function usage(input, output) {
  ctx.emit({
    kind: "usage",
    input_tokens: input || 0,
    output_tokens: output || 0,
  });
}
function prepareNative(request, plan) {
  const root = request.working_directory + "/.meerkat/" + harness.id;
  const directory = root + "/native-home";
  ctx.prepare({ ...plan, directory, environment: request.config.environment });
  if (plan.skills) ctx.skills(plan.skills);
  const env = { [plan.sourceVariable]: directory };
  for (const name of plan.directoryVariables || []) env[name] = directory;
  return { root, directory, env };
}
