/** QuickJS globals. No Node.js API, shell expansion, or vendor SDK is required. */
type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
interface HarnessConfig {
  harness_id: string;
  cli_path: string;
  launch_arguments: string[];
  environment: Record<string, string>;
  model: { model_id: string; display_name: string };
  thinking_level: string;
  config_fingerprint: string;
}
interface ToolDefinition {
  name: string;
  description: string;
  input_schema: Json;
  output_schema: Json;
}
interface SessionRequest {
  config: HarnessConfig;
  working_directory: string;
  system_prompt: string;
  tools: ToolDefinition[];
  native_session_id: string | null;
  permission_policy: string;
}
interface ToolResult {
  tool_call_id: string;
  success: boolean;
  content_markdown: string;
  structured_content: Json;
  resources: Json[];
  error: { code: string; message: string; retryable: boolean } | null;
}
interface Packet {
  process: number;
  message: Record<string, any>;
  exit?: boolean;
  detail?: string;
  mcp?: boolean;
}
interface NativeModel {
  id: string;
  name: string;
  description: string;
  thinking_levels: string[];
}
interface HarnessPlugin {
  apiVersion: 1;
  id: string;
  displayName: string;
  defaults?: {
    environment?: Record<string, string>;
    launch_arguments?: string[];
  };
  probe(config: HarnessConfig): {
    info: { harness_id: string; display_name: string };
    availability: "available" | "unavailable" | "unknown";
    version: string | null;
    capabilities: string[];
    reason: string | null;
  };
  models(config: HarnessConfig): NativeModel[];
  start(request: SessionRequest): { native_session_id: string };
  resume(request: SessionRequest): { native_session_id: string };
  execute(request: { input: string }): void;
  steer(request: { input: string }): void;
  toolResult(result: ToolResult): void;
  cancel(request: { reason: string }): { accepted: boolean } | void;
  close(): void;
  onMessage?(packet: Packet): void;
  onExit?(packet: Packet): void;
  onToolCall?(call: {
    tool_call_id: string;
    tool_name: string;
    arguments: Json;
  }): void;
  /** Invoked only by the explicit test API, before starting an isolated session. */
  test?(request: SessionRequest): { request: SessionRequest; input: string };
  classifyError?(error: any): Json;
}
declare function defineHarness(plugin: HarnessPlugin): void;
declare const ctx: {
  directory(): string;
  env(name: string): string | null;
  uuid(): string;
  fail(error: Json): void;
  cancelAfter(reason: string, millis?: number): void;
  write(path: string, content: string): void;
  canonical(path: string): string;
  read(path: string): string;
  /** Read a UTF-8 first line, including its newline; reject lines over 64 KiB. */
  readFirstLine(path: string): string;
  exists(path: string): boolean;
  list(path: string): string[];
  prepare(plan: {
    directory: string;
    sourceVariable: string;
    sourceRelative: string;
    pathKeys?: string[];
    files: string[];
    resources: string[];
    environment?: Record<string, string>;
  }): string;
  skills(path: string): void;
  spawn(options: {
    executable: string;
    args: string[];
    env?: Record<string, string | null>;
    cwd?: string;
    stderr?: string;
    record?: { path: string; limit: number };
  }): number;
  send(process: number, message: Json): void;
  receive(timeout: number): Packet | null;
  terminate(process: number): void;
  closeStdin(process: number): void;
  bindSession(id: string): void;
  emit(event: { kind: string; [key: string]: any }): void;
  mcp(directory: string, tools: (ToolDefinition & { _meta?: Json })[]): string;
  mcpResult(result: ToolResult): void;
};
/** Dispatches unrelated events while waiting for the response to one RPC. */
declare function receiveUntil(
  process: number,
  predicate: (message: any) => boolean,
  timeout?: number,
): any;
declare function launch(
  config: HarnessConfig,
  args: string[],
  options?: Record<string, any>,
): number;
declare function failure(code: string, message: string): never;
/** Refresh a private config overlay using the adapter's native directory rules. */
declare function prepareNative(
  request: Pick<SessionRequest, "working_directory" | "config">,
  plan: {
    sourceVariable: string;
    sourceRelative: string;
    files: string[];
    resources: string[];
    pathKeys?: string[];
    directoryVariables?: string[];
    skills?: string;
  },
): { root: string; directory: string; env: Record<string, string> };
