// Sample content lives only in the design document. The renderer is the unchanged app.
import type {
  Agent,
  Workspace,
  MachineCatalogItem,
  HarnessCatalogItem,
  ModelCatalogItem,
  HumanTeamMember,
} from "../../../apps/web/src/lib/api/generated";
import { avatarUrl } from "../../../apps/web/src/lib/assets";
const stamp = 1792454400;
const levels = ["low", "medium", "high", "xhigh"];
const harnesses: HarnessCatalogItem[] = [
  {
    harness_id: "builtin",
    display_name: "Meerkat",
    description: "",
    availability: "available",
    supported_thinking_levels: levels,
  },
  {
    harness_id: "codex",
    display_name: "Codex",
    description: "",
    availability: "available",
    supported_thinking_levels: levels,
  },
  {
    harness_id: "claude_code",
    display_name: "Claude Code",
    description: "",
    availability: "available",
    supported_thinking_levels: levels,
  },
  {
    harness_id: "pi",
    display_name: "Pi",
    description: "",
    availability: "available",
    supported_thinking_levels: levels,
  },
];
const catalogs: Record<string, [string, string][]> = {
  builtin: [
    ["gpt-6-luna", "GPT-6 Luna"],
    ["gpt-6-sol", "GPT-6 Sol"],
    ["gpt-6.1-sol", "GPT-6.1 Sol"],
  ],
  codex: [
    ["gpt-5.5", "GPT-5.5"],
    ["gpt-5.5-mini", "GPT-5.5 Mini"],
    ["gpt-5.4", "GPT-5.4"],
  ],
  claude_code: [
    ["claude-opus-5-5", "Claude Opus 5.5"],
    ["claude-sonnet-5-5", "Claude Sonnet 5.5"],
    ["claude-haiku-5-5", "Claude Haiku 5.5"],
  ],
  pi: [
    ["deepseek/deepseek-v4-flash", "DeepSeek Flash"],
    ["anthropic/claude-opus-5-5", "Claude Opus 5.5"],
    ["openai/gpt-5.5", "GPT-5.5"],
    ["google/gemini-3-pro", "Gemini 3 Pro"],
  ],
};
const models = (harness: string) =>
  (catalogs[harness] ?? []).map(([model_id, display_name]) => ({
    model_id,
    display_name,
    description: "",
    available: true,
    supported_thinking_levels: levels,
  }));
const machines: MachineCatalogItem[] = [
  ["mbp", "MacBook Pro", "running"],
  ["mini", "Mac mini", "running"],
  ["imac", "办公室 iMac", "stopped"],
].map(([machine_id, display_name, observed_state], i) => ({
  machine_id,
  display_name,
  observed_state,
  is_local: i === 0,
  registered: true,
  revoked_at: null,
  last_seen_at: stamp,
  paired_at: stamp,
  os: "macos",
  version: "0.1.0",
  status_message: "",
  harnesses: harnesses.map((h) => ({
    harness_id: h.harness_id,
    display_name: h.display_name,
    availability: h.availability,
    version: "",
    reason: "",
    model_catalog: { models: models(h.harness_id) as ModelCatalogItem[] },
    model_catalog_error: "",
    capabilities: [],
    // Only the Mac mini starts signed in, so the preview shows both states.
    ...(h.harness_id === "builtin"
      ? {
          account:
            machine_id === "mini"
              ? { method: "chatgpt", label: "lin@example.com" }
              : null,
        }
      : {}),
  })),
})) as MachineCatalogItem[];
// Sign-ins finish on their own a few seconds after the browser step opens.
const signIns = new Map<
  string,
  { machine_id: string; done_at: number; account: unknown; reason: string | null }
>();
const setAccount = (machine_id: string, account: unknown) => {
  const entry = machines
    .find((m) => m.machine_id === machine_id)
    ?.harnesses.find((h) => h.harness_id === "builtin");
  if (!entry) return;
  const key = (account as { method?: string } | null)?.method === "api_key";
  const list = key
    ? ["deepseek-v4-pro", "deepseek-flash"].map((model_id) => ({
        model_id,
        display_name: "",
        description: "",
        available: true,
        supported_thinking_levels: levels,
      }))
    : models("builtin");
  Object.assign(entry, { account, model_catalog: { models: list } });
};
const workspaces: Workspace[] = [
  ["product", "产品组", "feishu"],
  ["lab", "实验室", "telegram"],
].map(([id, name, provider_id]) => ({
  workspace: { workspace_id: id },
  name,
  provider_id,
  cover: null,
  rules:
    id === "product"
      ? "回复保持简短，先给结论。\n需求不明确时先追问，不要猜。\n任务完成后在清单里勾掉。"
      : "",
  provider_config_json: JSON.stringify({
    task_system_enabled: true,
    owner_person_id: "lead",
  }),
  desired_state: "running",
  observed_state: "running",
  status_message: "",
  created_at: stamp,
  updated_at: stamp,
})) as Workspace[];
const crew = [
  [
    "1",
    "小王",
    "产品经理",
    "01",
    "codex",
    "gpt-5.5",
    "mbp",
    "把模糊的想法拆成清楚的需求。",
    "OnIt",
  ],
  [
    "2",
    "阿青",
    "前端工程师",
    "11",
    "claude_code",
    "claude-sonnet-5-5",
    "mini",
    "喜欢把界面做得又快又稳。",
    "THUMBSUP",
  ],
  [
    "3",
    "米粒",
    "设计师",
    "04",
    "claude_code",
    "claude-opus-5-5",
    "mbp",
    "",
    "GLANCE",
  ],
  [
    "4",
    "阿哲",
    "数据分析",
    "14",
    "pi",
    "deepseek/deepseek-v4-flash",
    "mini",
    "用数字说话。",
    "Get",
  ],
  ["5", "老周", "测试", "07", "codex", "gpt-5.5-mini", "imac", "", "DONE"],
];
const agents: Agent[] = crew.map(
  (
    [id, name, role, av, harness_id, model, machine_id, bio, auto_reaction],
    i,
  ) => ({
    agent: { workspace_id: "product", agent_id: id },
    name,
    role,
    bio,
    avatar_path: av,
    figure: `builtin:${av}`,
    harness_id,
    model,
    thinking_level: "medium",
    machine_id,
    auto_reaction,
    soul:
      i === 0
        ? "动手之前，先问清楚要解决谁的什么问题。回复先给结论，再给理由。"
        : "",
    launch_arguments: [],
    cli_path: "",
    environment_variables: [],
    activity: i === 0 ? "busy" : "idle",
    pause_reason: i === 4 ? "连续 3 次没有回复" : null,
    attention: null,
    provider_agent_config_json: "{}",
    created_at: stamp + i,
    updated_at: stamp,
  }),
) as Agent[];
const humans: HumanTeamMember[] = [
  {
    person_id: "lead",
    team_role: "owner",
    name: "林小满",
    avatar: null,
    position: "产品负责人",
    signature: "把有用的事情做好。",
  },
];
const reactions = [
  "OnIt",
  "Get",
  "THUMBSUP",
  "OK",
  "DONE",
  "CheckMark",
  "LGTM",
  "GLANCE",
  "Typing",
  "OneSecond",
  "THINKING",
  "SMILE",
  "CLAP",
  "APPLAUSE",
  "Fire",
  "Hundred",
  "MUSCLE",
  "FISTBUMP",
  "HIGHFIVE",
  "THANKS",
  "HEART",
  "PARTY",
  "WOW",
  "Trophy",
  "SALUTE",
  "Coffee",
  "Pin",
  "Alarm",
  "MeMeMe",
  "YEAH",
  "GoGoGo",
];
let taskName = "产品组任务";
const tasks = [
  "整理 10 月版本需求",
  "首页改版走查",
  "修复登录页闪烁",
  "周活跃数据看板",
  "2.3 回归测试",
].map((text, i) => ({ id: String(i), text, completed: i === 2 || i === 4 }));
const progress = new Map<string, number>();
const phaseTicks = Number(new URLSearchParams(location.search).get("scan") ?? 8);
const uploaded = new Map<string, { bytes: string; media_type: string }>();
const originalFetch = globalThis.fetch.bind(globalThis);
const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
const copy = <T>(target: T, patch: Record<string, unknown>) => {
  for (const [key, value] of Object.entries(patch))
    if (value !== null && key in (target as object))
      (target as Record<string, unknown>)[key] = value;
  return target;
};
// All writes stay in this tab. Unknown operations fail rather than reach a real service.
globalThis.fetch = async (input, init) => {
  const path =
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url;
  if (!path.startsWith("/api/")) return originalFetch(input, init);
  const op = path.split("/").pop();
  const data = JSON.parse(String(init?.body ?? "{}")) ?? {};
  const wid =
    data.workspace?.workspace_id ?? data.agent?.workspace_id ?? "product";
  const team = workspaces.find((w) => w.workspace?.workspace_id === wid)!;
  const agent = agents.find(
    (a) =>
      a.agent?.agent_id === data.agent?.agent_id &&
      a.agent?.workspace_id === wid,
  );
  const envelope = { request_id: data.request_id, workspace: data.workspace };
  switch (op) {
    case "list_workspaces":
      return json({ ...envelope, workspaces });
    case "get_workspace":
      return json(team);
    case "create_workspace": {
      const made = {
        ...workspaces[0],
        workspace: { workspace_id: data.workspace_id },
        name: data.name,
        rules: data.rules,
        provider_id: data.provider_id,
        provider_config_json: data.provider_config_json,
      };
      workspaces.push(made);
      return json(made);
    }
    case "list_agents":
      return json({
        ...envelope,
        agents: agents.filter((a) => a.agent?.workspace_id === wid),
      });
    case "get_agent":
      return agent
        ? json(agent)
        : json({ code: "not_found", message: "", detail: null }, 404);
    case "get_agent_avatar": {
      const uploadedFile = uploaded.get(data.agent.agent_id);
      const image =
        uploadedFile ??
        (agent?.avatar_path
          ? await originalFetch(avatarUrl(agent.avatar_path)!).then(
              async (r) => ({
                media_type: "image/jpeg",
                bytes: btoa(
                  String.fromCharCode(...new Uint8Array(await r.arrayBuffer())),
                ),
              }),
            )
          : null);
      return json({
        ...envelope,
        agent: data.agent,
        avatar: image ? { ...image, path: agent?.avatar_path ?? "" } : null,
      });
    }
    case "get_machine_catalog":
      return json({ machines });
    case "get_harness_catalog":
      return json({ harnesses });
    case "builtin_agent_sign_in": {
      const sign_in_id = `s${signIns.size + 1}`;
      signIns.set(sign_in_id, {
        machine_id: data.machine_id,
        done_at: Date.now() + (data.method === "api_key" ? 1200 : 4000),
        account:
          data.method === "api_key"
            ? { method: "api_key", label: new URL(data.base_url).host }
            : { method: "chatgpt", label: "lin@example.com" },
        reason: data.method === "api_key" && data.api_key === "bad" ? "invalid_key" : null,
      });
      if (data.method === "api_key") return json({ sign_in_id, authorize_url: null });
      return json({ sign_in_id, authorize_url: "about:blank" });
    }
    case "builtin_agent_sign_in_status": {
      const sign_in = signIns.get(data.sign_in_id);
      if (!sign_in) return json({ state: "expired", reason: null });
      if (Date.now() < sign_in.done_at)
        return json({ state: "pending", reason: null });
      if (sign_in.reason) return json({ state: "failed", reason: sign_in.reason });
      setAccount(sign_in.machine_id, sign_in.account);
      return json({ state: "succeeded", reason: null });
    }
    case "builtin_agent_sign_in_cancel":
      signIns.delete(data.sign_in_id);
      return json(null);
    case "builtin_agent_sign_out":
      setAccount(data.machine_id, null);
      return json(null);
    case "get_model_catalog": {
      // A key account lists what that service offers.
      const account = (
        machines
          .find((m) => m.machine_id === data.machine_id)
          ?.harnesses.find((h) => h.harness_id === "builtin") as
          | { account?: { method: string; label: string } | null }
          | undefined
      )?.account;
      if (data.harness_id === "builtin" && account?.method === "api_key")
        return json({
          models: [
            ["deepseek-v4-pro", ""],
            ["deepseek-flash", ""],
          ].map(([model_id]) => ({
            model_id,
            display_name: "",
            description: "",
            available: true,
            supported_thinking_levels: levels,
          })),
        });
      return json({ models: models(data.harness_id) });
    }
    case "get_reaction_catalog":
      return json({
        reactions: reactions.map((reaction_id) => ({
          reaction_id,
          display_name: reaction_id,
          emoji: "",
          available: true,
        })),
      });
    case "list_figure_packs":
      return json([]);
    case "list_human_members":
      return json(wid === "product" ? humans : []);
    case "sync_human_profile":
      return json(humans.find((h) => h.person_id === data.person_id));
    case "update_human_position": {
      const human = humans.find((h) => h.person_id === data.person_id)!;
      human.position = data.position;
      return json(humans);
    }
    case "update_agent": {
      if (!agent)
        return json({ code: "not_found", message: "", detail: null }, 404);
      copy(agent, data);
      if (data.environment_variables)
        agent.environment_variables = data.environment_variables.entries;
      if (data.avatar) {
        uploaded.set(data.agent.agent_id, data.avatar);
        agent.avatar_path = "uploaded";
      }
      return json(agent);
    }
    case "create_agent": {
      const made = {
        ...agents[0],
        ...data,
        activity: "idle",
        avatar_path: "",
        pause_reason: null,
      };
      agents.push(made);
      return json(made);
    }
    case "delete_agent": {
      if (agent) agents.splice(agents.indexOf(agent), 1);
      return json({ ...envelope, agent: data.agent });
    }
    case "resume_agent": {
      if (agent) agent.pause_reason = null;
      return json(agent);
    }
    case "update_workspace":
      return json(copy(team, data));
    case "set_workspace_state":
      team.desired_state = data.desired_state;
      team.observed_state = data.desired_state;
      return json(team);
    case "invoke_provider_interface": {
      const action = JSON.parse(data.input_json);
      if (action.action === "rename") taskName = action.name;
      return json({
        ...envelope,
        interface_id: data.interface_id,
        output_json: JSON.stringify({
          enabled: JSON.parse(team.provider_config_json).task_system_enabled,
          name: taskName,
          items: tasks,
        }),
      });
    }
    case "list_provider_interfaces":
      return json({ ...envelope, interfaces: [] });
    case "rename_machine": {
      const m = machines.find((m) => m.machine_id === data.machine_id)!;
      m.display_name = data.name;
      return json({ machines });
    }
    case "revoke_machine": {
      const m = machines.find((m) => m.machine_id === data.machine_id)!;
      m.revoked_at = stamp;
      return json({ machines });
    }
    case "enable_remote_access":
    case "get_remote_access":
    case "set_remote_access":
      return json({
        address: data.address || "127.0.0.1:7443",
        cert_sha256: "0".repeat(64),
      });
    case "create_pairing_code":
      return json({
        code: "LK-PREVIEW",
        issued_at: Date.now(),
        expires_at: Date.now() + 600_000,
      });
    case "advance_agent_onboarding":
    case "get_agent_onboarding": {
      // New members walk through the official registration, then (Feishu)
      // permissions, a few seconds each, so the connect step can be reviewed.
      const key = data.agent?.agent_id ?? "";
      if (data.action === "start" && !progress.has(key)) progress.set(key, 0);
      const ticks = progress.get(key);
      if (ticks !== undefined && data.action === "poll") progress.set(key, ticks + 1);
      const at = progress.get(key);
      const feishu = team?.provider_id !== "telegram";
      const phase =
        at === undefined
          ? "ready"
          : at < phaseTicks
            ? "create_app"
            : feishu && at < phaseTicks * 2
              ? "permissions"
              : "ready";
      return json({
        agent: data.agent,
        provider_id: team?.provider_id,
        step: phase,
        status: phase === "ready" ? "ready" : "action_required",
        payload_json: "{}",
        verification_uri:
          phase === "create_app"
            ? "https://open.feishu.cn/page/launcher?user_code=PREVIEW"
            : phase === "permissions"
              ? "https://open.feishu.cn/app/cli_preview/auth?q=im:message"
              : "",
        poll_interval_seconds: 1,
        error_code: "",
        error_message: "",
      });
    }
    case "get_provider_catalog":
      return json({
        providers: [
          {
            provider_id: "feishu",
            display_name: "飞书",
            description: "",
            available: true,
            capabilities: [],
          },
          {
            provider_id: "telegram",
            display_name: "Telegram",
            description: "",
            available: true,
            capabilities: [],
          },
        ],
      });
    case "get_agent_first_reply":
      return json({ state: "replied" });
    case "test_harness":
      return json({ state: "passed", reply: "你好！" });
    default:
      console.error("Unhandled design operation", op);
      return json({ code: "unimplemented", message: "", detail: null }, 501);
  }
};

// Preserve the separate onboarding storyboard's handoff into the main sample.
export function addStoryboardMember(member: Record<string, unknown>) {
  if (typeof member.name !== "string" || !member.name.trim()) return;
  agents.push({
    ...agents[0],
    agent: { workspace_id: "product", agent_id: "story-" + String(member.id) },
    name: member.name.trim(),
    role: String(member.role ?? ""),
    bio: String(member.bio ?? ""),
    soul: String(member.soul ?? ""),
    avatar_path: String(member.av ?? "00").padStart(2, "0"),
    figure: "builtin:" + String(member.av ?? "00").padStart(2, "0"),
    harness_id:
      member.engine === "claude"
        ? "claude_code"
        : String(member.engine ?? "codex"),
    model: String(member.model ?? "gpt-5.5"),
    machine_id: String(member.machine ?? "mbp"),
    auto_reaction: String(member.reaction ?? "OnIt"),
    activity: "idle",
    pause_reason: null,
  });
  dispatchEvent(new Event("meerkat-design-data"));
}
