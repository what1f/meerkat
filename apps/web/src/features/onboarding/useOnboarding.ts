import { useRef, useState } from 'react';
import { useT } from '../../lib/i18n';
import { useCreateWorkspace, useUpdateWorkspace } from '../../lib/queries/workspaces';
import { useAgent, useAgentFirstReply } from '../../lib/queries/agents';
import './messages';

export type Provider = 'feishu' | 'telegram';
// Welcome, team, first member, ready.
export type Step = 0 | 1 | 2 | 3;
const request_id = 'onboarding';
const newId = (prefix: string, length: number) =>
  prefix + crypto.randomUUID().replaceAll('-', '').slice(0, length);

// The team (name and chat app) is created on leaving its step. The first member is
// then made on the same bench as every later member, and the flow ends at their reply.
export function useOnboarding(onFinish: (workspace?: string) => void) {
  const t = useT();
  const [step, setStep] = useState<Step>(0);
  const [provider, setProviderState] = useState<Provider>('feishu');
  // A late answer must not move on a flow the person has left.
  const current = useRef<Provider>('feishu');
  const [workspace, setWorkspace] = useState<string>();
  const [teamName, setTeamNameState] = useState(() => t('onboarding.teamName'));
  const [token, setTokenState] = useState('');
  const [tokenInvalid, setTokenInvalid] = useState(false);
  const [agentId, setAgentId] = useState<string>();
  const createWorkspace = useCreateWorkspace();
  const renameWorkspace = useUpdateWorkspace('onboarding_team');
  const agent = { request_id, agent: { workspace_id: workspace ?? '', agent_id: agentId ?? '' } };
  const firstReply = useAgentFirstReply(agent, step === 3);
  const replied = firstReply.data?.state === 'replied';
  const member = useAgent(agent, step === 3).data;
  const name = () => teamName.trim() || t('onboarding.teamName');

  function create(provider_id: Provider, secrets: object, onDone: (workspace: string) => void) {
    const workspace_id = newId('t', 8);
    createWorkspace.mutate(
      {
        request_id,
        workspace_id,
        name: name(),
        rules: '',
        provider_id,
        provider_config_json: JSON.stringify(
          provider_id === 'feishu' ? { domain: 'feishu', task_system_enabled: false } : {},
        ),
        provider_secrets_json: JSON.stringify(secrets),
      },
      {
        onSuccess: () => {
          setWorkspace(workspace_id);
          if (current.current === provider_id) onDone(workspace_id);
        },
        onError: () => provider_id === 'telegram' && setTokenInvalid(true),
      },
    );
  }
  function withTeam(then: (workspace: string) => void) {
    if (workspace) return then(workspace);
    if (createWorkspace.isPending) return;
    create(provider, provider === 'telegram' ? { manager_bot_token: token.trim() } : {}, then);
  }
  const finish = () => onFinish(workspace);
  // Leaving early still leaves a team; Telegram without a token falls back to Feishu.
  const leave = () => {
    if (workspace) return finish();
    if (provider === 'telegram' && !token.trim()) {
      current.current = 'feishu';
      return create('feishu', {}, (id) => onFinish(id));
    }
    withTeam((id) => onFinish(id));
  };
  const next = () => {
    if (step === 0) setStep(1);
    else if (step === 1) withTeam(() => setStep(2));
    else finish();
  };
  const linked = provider === 'feishu' || !!workspace || token.trim() !== '';
  return {
    step,
    provider,
    // The chat app is fixed once the team exists.
    locked: !!workspace || createWorkspace.isPending,
    setProvider: (value: Provider) => {
      current.current = value;
      setProviderState(value);
    },
    workspace,
    teamName,
    // Still the name it came with.
    teamNamed: teamName.trim() !== '' && teamName !== t('onboarding.teamName'),
    setTeamName: (value: string) => {
      setTeamNameState(value);
      if (workspace && value.trim())
        renameWorkspace.mutate({
          request_id: 'onboarding_team_name',
          workspace: { workspace_id: workspace },
          name: value.trim(),
          rules: null,
          provider_config_json: null,
          cover_resource_id: null,
          provider_secrets_json: null,
        });
    },
    token,
    setToken: (value: string) => {
      setTokenState(value);
      setTokenInvalid(false);
    },
    tokenInvalid,
    // The member has joined the chat app: on to their first reply.
    joined: (id: string) => {
      setAgentId(id);
      setStep(3);
    },
    member,
    replied,
    reply: {
      failed: firstReply.isError && !replied,
      retry: () => void firstReply.refetch(),
    },
    foot: {
      // The member step is driven from its console.
      ready: step === 1 ? linked && !createWorkspace.isPending : step === 3 ? replied : false,
      primary: step !== 2,
      loading: step === 1 && createWorkspace.isPending,
      next,
      later: leave,
      back: step === 1 || step === 2 ? () => setStep((s) => (s - 1) as Step) : undefined,
    },
  };
}
export type Flow = ReturnType<typeof useOnboarding>;
