import { useRef, useState } from 'react';
import { useToast } from '../../design';
import { useT } from '../../lib/i18n';
import { useCreateWorkspace } from '../../lib/queries/workspaces';
import { useAgent, useAgentFirstReply, useCreateAgent } from '../../lib/queries/agents';
import { useAgentOnboardingProgress, useStartAgentOnboarding } from '../../lib/queries/onboarding';
import { firstModel, useAiPick } from './useAiPick';
import './messages';

export type Provider = 'feishu' | 'telegram';
export type Step = 0 | 1 | 2 | 3 | 4;
export const memberFigure = '15';
const request_id = 'onboarding';
const newId = (prefix: string, length: number) =>
  prefix + crypto.randomUUID().replaceAll('-', '').slice(0, length);

// Steps: welcome, connect, AI, first member, ready. Skipping a step also skips
// the steps that need it: without a connection there is no member to create.
export function useOnboarding(onFinish: (workspace?: string) => void) {
  const t = useT();
  const toast = useToast();
  const [raw, setStep] = useState<Step>(0);
  const [provider, setProviderState] = useState<Provider>('feishu');
  // A late answer must not move on a flow the person has left.
  const current = useRef<Provider>('feishu');
  const [teams, setTeams] = useState<Partial<Record<Provider, string>>>({});
  const [offline, setOffline] = useState(false);
  const [token, setTokenState] = useState('');
  const [tokenInvalid, setTokenInvalid] = useState(false);
  const [draft, setDraft] = useState({ name: '', role: '', soul: '' });
  const [agentId, setAgentId] = useState<string>();
  const createWorkspace = useCreateWorkspace();
  const createAgent = useCreateAgent();
  const startMember = useStartAgentOnboarding();
  const workspace = teams[provider];
  const ai = useAiPick(request_id);
  const agent = { request_id, agent: { workspace_id: workspace ?? '', agent_id: agentId ?? '' } };
  const progressQuery = useAgentOnboardingProgress(agent, !!agentId);
  const progress = progressQuery.data;
  const step: Step = raw === 3 && progress?.status === 'ready' ? 4 : raw;
  const firstReply = useAgentFirstReply(agent, step === 4);
  const replied = firstReply.data?.state === 'replied';
  const member = useAgent(agent, step === 4).data;
  const connected = provider === 'feishu' ? !!teams.feishu : !!teams.telegram;
  const failed = progress?.status === 'error' || progressQuery.isError || startMember.isError;
  const creating =
    createAgent.isPending || startMember.isPending || (!!agentId && !!progress && !failed);

  function create(provider_id: Provider, secrets: object, onDone?: () => void) {
    const workspace_id = newId('t', 8);
    createWorkspace.mutate(
      {
        request_id,
        workspace_id,
        name: t('onboarding.teamName'),
        rules: '',
        provider_id,
        provider_config_json: JSON.stringify(
          provider_id === 'feishu' ? { domain: 'feishu', task_system_enabled: false } : {},
        ),
        provider_secrets_json: JSON.stringify(secrets),
      },
      {
        onSuccess: () => {
          setTeams((all) => ({ ...all, [provider_id]: workspace_id }));
          if (current.current === provider_id) onDone?.();
        },
        onError: () => provider_id === 'telegram' && setTokenInvalid(true),
      },
    );
  }
  const ensureFeishu = () => {
    if (!teams.feishu && !createWorkspace.isPending) create('feishu', {});
  };
  function enterAi() {
    setStep(2);
    ai.enter();
  }
  function createMember() {
    const { machine, pick, picked } = ai;
    if (!workspace || !machine || !pick) return;
    if (agentId) return startMember.mutate(agent);
    const agent_id = newId('m', 6);
    const ref = { request_id, agent: { workspace_id: workspace, agent_id } };
    const config =
      provider === 'feishu'
        ? { mode: 'new', domain: 'feishu' }
        : { mode: 'new', bot_username: `${workspace}_${agent_id}_bot` };
    createAgent.mutate(
      {
        ...ref,
        ...firstModel(picked),
        name: draft.name.trim(),
        role: draft.role.trim(),
        bio: '',
        figure: `builtin:${memberFigure}`,
        launch_arguments: [],
        harness_id: pick,
        auto_reaction: '',
        soul: draft.soul,
        cli_path: '',
        environment_variables: [],
        machine_id: machine.machine_id,
        attention: null,
        provider_agent_config_json: JSON.stringify(config),
        provider_agent_secrets_json: '{}',
      },
      {
        onSuccess: () => {
          setAgentId(agent_id);
          startMember.mutate(ref);
        },
        onError: () => toast({ message: t('onboarding.createFailed') }),
      },
    );
  }
  const finish = () => onFinish(workspace ?? teams.feishu ?? teams.telegram);
  // No AI to create the member with: nothing is left to set up.
  const afterAi = () => (offline || !ai.machine || !ai.pick ? finish() : setStep(3));
  const next = () => {
    if (step === 0) {
      setStep(1);
      if (provider === 'feishu') ensureFeishu();
    } else if (step === 1) {
      if (provider === 'feishu') enterAi();
      else if (teams.telegram) enterAi();
      else create('telegram', { manager_bot_token: token.trim() }, enterAi);
    } else if (step === 2) afterAi();
    else if (step === 3) createMember();
    else finish();
  };
  const later = () => {
    if (step === 1) {
      setOffline(true);
      enterAi();
    } else if (step === 2) afterAi();
    else finish();
  };
  const ready = {
    1: connected || (provider === 'telegram' && token.trim() !== ''),
    2: ai.available.length > 0,
    3: !!draft.name.trim() && !creating && !!ai.machine && !!ai.pick,
    4: replied,
  }[step as 1 | 2 | 3 | 4];
  return {
    step,
    provider,
    setProvider: (value: Provider) => {
      current.current = value;
      setProviderState(value);
      if (value === 'feishu') ensureFeishu();
    },
    workspace,
    token,
    setToken: (value: string) => {
      setTokenState(value);
      setTokenInvalid(false);
    },
    tokenInvalid,
    ai,
    draft,
    setDraft,
    creating,
    failed,
    confirm:
      progress?.status !== 'ready' && progress?.status !== 'error'
        ? progress?.verification_uri
        : '',
    permissions: progress?.step === 'permissions',
    refreshQr: () => startMember.mutate(agent),
    member,
    replied,
    reply: {
      failed: firstReply.isError && !replied,
      retry: () => void firstReply.refetch(),
    },
    foot: { ready, loading: step === 1 && createWorkspace.isPending, next, later },
  };
}
export type Flow = ReturnType<typeof useOnboarding>;
