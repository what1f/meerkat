import { useRef, useState } from 'react';
import { ApiError } from '../../lib/api/client';
import { useAgent, useAgentAvatar, useCreateAgent, useUpdateAgent } from '../../lib/queries/agents';
import { useHarnessCatalog, useModelCatalog } from '../../lib/queries/catalogs';
import { useMachines } from '../../lib/queries/machines';
import { useAgentOnboardingProgress, useStartAgentOnboarding } from '../../lib/queries/onboarding';
import { avatarUrl } from '../../lib/assets';
import { builtinUpload, defaultAvatar } from './avatars';
import { creationOf, onboards, type Creation } from './creation';
import { formatArgs, parseArgs } from './args';
import {
  blank,
  broken,
  dataUrl,
  oneAtATime,
  emptyUpdate,
  fields,
  newId,
  pick,
  same,
  type AvatarUpload,
  type Draft,
} from './draft';

export function useMemberDraft(workspace: string, id: string | null, provider?: string) {
  const ref = { workspace_id: workspace };
  const agentRef = id ? { workspace_id: workspace, agent_id: id } : null;
  const agent = useAgent({ request_id: 'member', agent: agentRef }, !!id);
  const avatar = useAgentAvatar({ request_id: 'member', agent: agentRef }, !!id);
  const machines = useMachines({ request_id: 'member', workspace: ref });
  const harnesses = useHarnessCatalog({ request_id: 'member', workspace: ref });
  const [edits, setEdits] = useState<Partial<Draft>>({});
  // Launch arguments keep the text as typed; it is split into arguments only when sent.
  const [argsText, setArgsText] = useState<string | null>(null);
  const [upload, setUpload] = useState<AvatarUpload | null>(null);
  const create = useCreateAgent();
  const update = useUpdateAgent();
  const busy = useRef(false);
  const [newAgentId] = useState(newId);
  // Set as soon as the Runtime has the new member, so a retry never creates it twice.
  const [created, setCreated] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const start = useStartAgentOnboarding();
  const onboarding = useAgentOnboardingProgress(
    {
      request_id: 'member',
      agent: pending ? { workspace_id: workspace, agent_id: pending } : null,
    },
    !!pending,
  );

  const base: Draft = agent.data ? { ...blank, ...pick(agent.data) } : blank;
  const merged = { ...base, ...edits };
  if (argsText !== null) merged.launch_arguments = parseArgs(argsText);
  // A new member starts on the local Machine with its first available AI.
  const list = machines.data?.machines.filter((m) => m.revoked_at === null) ?? [];
  const machine =
    list.find((m) => m.machine_id === merged.machine_id) ??
    (id ? undefined : (list.find((m) => m.is_local) ?? list[0]));
  const harness =
    merged.harness_id ||
    (
      harnesses.data?.harnesses.find((h) => h.availability === 'available') ??
      harnesses.data?.harnesses[0]
    )?.harness_id ||
    '';
  const draft: Draft = { ...merged, machine_id: machine?.machine_id ?? merged.machine_id };
  draft.harness_id = harness;
  const models = useModelCatalog(
    { request_id: 'member', workspace: ref, machine_id: draft.machine_id, harness_id: harness },
    !!draft.machine_id && !!harness,
  );
  if (!id && !draft.model) {
    const first = models.data?.models.find((m) => m.available) ?? models.data?.models[0];
    draft.model = first?.model_id ?? '';
    draft.thinking_level ||= first?.supported_thinking_levels[0] ?? '';
  }

  const changed = fields.filter((key) => !same(draft[key], base[key]));
  const dirty = !!upload || (id ? changed.length > 0 : true);
  const set = (patch: Partial<Draft>) => setEdits((current) => ({ ...current, ...patch }));
  const reset = () => {
    setEdits({});
    setArgsText(null);
    setUpload(null);
  };
  const avatarSrc = upload
    ? dataUrl(upload)
    : avatar.data?.avatar
      ? dataUrl(avatar.data.avatar)
      : id
        ? undefined
        : avatarUrl(defaultAvatar(newAgentId));
  const creation: Creation = pending
    ? start.isPending
      ? { step: 'creating' }
      : creationOf(pending, onboarding.data ?? undefined, onboarding.isError || start.isError)
    : create.isPending
      ? { step: 'creating' }
      : { step: 'idle' };
  const notFound = agent.error instanceof ApiError && agent.error.code === 'not_found';
  const extras = [avatar, machines, harnesses, models].filter(broken);

  const once = <T>(write: () => Promise<T>) => oneAtATime(busy, write);
  const save = () =>
    once(async () => {
      if (!agentRef) return;
      const sent = Object.fromEntries(changed.map((key) => [key, draft[key]])) as Partial<Draft>;
      const [sentArgs, sentUpload] = [argsText, upload];
      const { environment_variables: entries, ...rest } = sent;
      await update.mutateAsync({
        ...emptyUpdate(),
        ...rest,
        environment_variables: entries ? { entries } : null,
        agent: agentRef,
        avatar: upload,
      });
      // Anything edited while the save was on its way stays as a change.
      setEdits((current) =>
        Object.fromEntries(
          Object.entries(current).filter(
            ([key, value]) => !(key in sent) || !same(value, sent[key as keyof Draft]),
          ),
        ),
      );
      setArgsText((current) => (current === sentArgs ? null : current));
      setUpload((current) => (current === sentUpload ? null : current));
    });
  // The picked avatar, or the built-in one chosen from the member id.
  async function putAvatar() {
    const picture = upload ?? (await builtinUpload(defaultAvatar(newAgentId)));
    if (picture)
      await update.mutateAsync({
        ...emptyUpdate(),
        agent: { workspace_id: workspace, agent_id: newAgentId },
        avatar: picture,
      });
    reset();
    return true;
  }
  // Resolves to the new id once the member can be opened, or null while its account is still being set up.
  const add = () =>
    once(async () => {
      const setup = onboards(provider);
      const createdRef = { workspace_id: workspace, agent_id: newAgentId };
      if (!created) {
        await create.mutateAsync({
          ...draft,
          request_id: crypto.randomUUID(),
          agent: createdRef,
          attention: null,
          provider_agent_config_json: setup ? '{"mode":"new","domain":"feishu"}' : '{}',
          provider_agent_secrets_json: '{}',
        });
        setCreated(true);
      }
      if (!setup) {
        await putAvatar();
        return newAgentId;
      }
      // The new app is set up by agent onboarding; the creation state follows it. Its
      // profile cannot take an avatar before that, so the avatar goes up when it is opened.
      setPending(newAgentId);
      start.mutate({ request_id: 'member', agent: createdRef });
      return null;
    });

  return {
    agent: agent.data,
    loading: !!id && agent.isPending && !broken(agent),
    notFound: !!id && notFound,
    failed: !!id && !notFound && broken(agent),
    retry: () => void agent.refetch(),
    retrying: agent.isFetching,
    partial: extras.length > 0,
    retryPartial: () => extras.forEach((query) => void query.refetch()),
    retryingPartial: extras.some((query) => query.isFetching),
    memberId: id ?? newAgentId,
    draft,
    argsText: argsText ?? formatArgs(draft.launch_arguments),
    setArgs: setArgsText,
    dirty,
    set,
    reset,
    save,
    add,
    // True once the avatar is up; false when another write is still on its way.
    finish: async () => (await once(putAvatar)) === true,
    saving: create.isPending || update.isPending,
    created,
    creation,
    avatarSrc,
    setUpload,
    machines: list,
    machine,
    models: models.data?.models ?? [],
    harnesses: harnesses.data?.harnesses ?? [],
  };
}
