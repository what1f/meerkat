"use strict";
let processId,
  nativeId,
  active = false,
  cancelling = false,
  sequence = 0;
const observed = new Set();
function open(request, resume) {
  processId = launch(request.config, mockOptions.arguments, {
    cwd: request.working_directory,
  });
  ctx.send(processId, { op: resume ? "resume" : "start", request });
  let response;
  try {
    response = receiveUntil(
      processId,
      (m) => m.type === "bound",
      mockOptions.bind_timeout,
    );
  } catch (error) {
    if (error.code === "stream_interrupted")
      failure("bind_failed", error.message);
    throw error;
  }
  if (!response.native_session_id)
    failure("bind_failed", "Peer did not bind a native session");
  nativeId = response.native_session_id;
  ctx.bindSession(nativeId);
  return { native_session_id: nativeId };
}
defineHarness({
  apiVersion: 1,
  id: mockOptions.id,
  displayName: mockOptions.name,
  probe(config) {
    processId = launch(config, mockOptions.arguments);
    try {
      ctx.send(processId, { op: "probe", config });
      const result = receiveUntil(
        processId,
        (m) => m.type === "probe_result",
        mockOptions.bind_timeout,
      );
      return {
        ...result,
        info: { harness_id: mockOptions.id, display_name: mockOptions.name },
      };
    } finally {
      ctx.terminate(processId);
    }
  },
  start: (r) => open(r, false),
  resume: (r) => open(r, true),
  execute(request) {
    active = true;
    sequence = 0;
    cancelling = false;
    observed.clear();
    ctx.send(processId, { op: "execute", request });
  },
  steer(value) {
    if (!active || cancelling)
      failure("invalid_state", "Steer requires an active execution");
    ctx.send(processId, { op: "steer", ...value });
  },
  toolResult(request) {
    if (!active || cancelling || !observed.has(request.tool_call_id))
      failure("invalid_state", "Tool call was not observed in this execution");
    ctx.send(processId, { op: "tool_result", request });
  },
  cancel(value) {
    if (cancelling) return { accepted: false };
    if (!active)
      failure("invalid_state", "Cancel requires an active execution");
    cancelling = true;
    ctx.send(processId, { op: "cancel", reason: value.reason });
    ctx.cancelAfter(value.reason, mockOptions.cancel_grace);
    return { accepted: true };
  },
  close() {
    if (active) {
      active = false;
      ctx.emit({ kind: "cancelled", reason: "Session closed" });
    }
    try {
      ctx.send(processId, { op: "close" });
    } catch {}
    ctx.terminate(processId);
  },
  onExit(packet) {
    if (packet.process === processId && active) {
      active = false;
      ctx.fail(
        classifyError({
          code: "stream_interrupted",
          message: "Harness event stream ended before a terminal event",
        }),
      );
    }
  },
  onMessage(packet) {
    if (packet.process !== processId || !active) return;
    const event = packet.message;
    if (event.invalid_output) failure("invalid_event", "Invalid native event");
    if (event.kind && event.sequence !== ++sequence)
      failure("invalid_event", "Native event sequence is not contiguous");
    if (event.kind === "tool_started") observed.add(event.tool_call_id);
    if (["completed", "cancelled", "failed"].includes(event.kind))
      active = false;
    if (event.kind) ctx.emit(event);
  },
});
