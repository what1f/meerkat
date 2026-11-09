import type * as DTO from './generated';
export class ApiError extends Error {
  constructor(
    public readonly code: DTO.ErrorCode,
    message: string,
    public readonly detail: string | null = null,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}
interface Operations {
  list_human_members: { input: DTO.GetWorkspaceRequest; output: DTO.HumanTeamMember[] };
  sync_human_profile: { input: DTO.SyncHumanProfileRequest; output: DTO.HumanTeamMember };
  update_human_position: { input: DTO.UpdateHumanPositionRequest; output: DTO.HumanTeamMember[] };
  list_timers: { input: DTO.GetWorkspaceRequest; output: DTO.Timer[] };
  cancel_timer: { input: DTO.TimerRequest; output: DTO.Timer };
  restore_timer: { input: DTO.TimerRequest; output: DTO.Timer };
  list_figure_packs: { input: DTO.ListFigurePacksRequest; output: DTO.FigurePack[] };
  import_figure_pack: { input: DTO.ImportFigurePackRequest; output: DTO.FigurePack };
  get_agent_first_reply: { input: DTO.GetAgentRequest; output: DTO.AgentFirstReply };
  test_harness: { input: DTO.TestHarnessRequest; output: DTO.TestHarnessResponse };
  list_workspaces: { input: DTO.ListWorkspacesRequest; output: DTO.ListWorkspacesResponse };
  get_workspace: { input: DTO.GetWorkspaceRequest; output: DTO.Workspace };
  get_workspace_image: { input: DTO.WorkspaceImageRequest; output: DTO.WorkspaceImageResponse };
  upload_workspace_image: {
    input: DTO.UploadWorkspaceImageRequest;
    output: DTO.UploadWorkspaceImageResponse;
  };
  create_workspace: { input: DTO.CreateWorkspaceRequest; output: DTO.Workspace };
  update_workspace: { input: DTO.UpdateWorkspaceRequest; output: DTO.Workspace };
  set_workspace_state: { input: DTO.SetWorkspaceStateRequest; output: DTO.Workspace };
  list_agents: { input: DTO.ListAgentsRequest; output: DTO.ListAgentsResponse };
  get_agent: { input: DTO.GetAgentRequest; output: DTO.Agent };
  cancel_attempt: { input: DTO.CancelAttemptRequest; output: DTO.CancelAttemptResponse };
  get_execution_details: { input: DTO.GetExecutionDetailsRequest; output: DTO.ExecutionDetails };
  get_agent_avatar: { input: DTO.GetAgentAvatarRequest; output: DTO.GetAgentAvatarResponse };
  create_agent: { input: DTO.CreateAgentRequest; output: DTO.Agent };
  update_agent: { input: DTO.UpdateAgentRequest; output: DTO.Agent };
  resume_agent: { input: DTO.GetAgentRequest; output: DTO.Agent };
  delete_agent: { input: DTO.DeleteAgentRequest; output: DTO.DeleteAgentResponse };
  get_provider_onboarding: {
    input: DTO.GetProviderOnboardingRequest;
    output: DTO.ProviderOnboardingState;
  };
  advance_provider_onboarding: {
    input: DTO.AdvanceProviderOnboardingRequest;
    output: DTO.ProviderOnboardingState;
  };
  get_agent_onboarding: { input: DTO.GetAgentOnboardingRequest; output: DTO.AgentOnboardingState };
  advance_agent_onboarding: {
    input: DTO.AdvanceAgentOnboardingRequest;
    output: DTO.AgentOnboardingState;
  };
  get_provider_catalog: { input: DTO.CatalogRequest; output: DTO.ProviderCatalog };
  get_machine_catalog: { input: DTO.CatalogRequest; output: DTO.MachineCatalog };
  create_pairing_code: { input: DTO.CatalogRequest; output: DTO.PairingCode };
  rename_machine: { input: DTO.RenameMachineRequest; output: DTO.MachineCatalog };
  revoke_machine: { input: DTO.RevokeMachineRequest; output: DTO.MachineCatalog };
  get_harness_catalog: { input: DTO.CatalogRequest; output: DTO.HarnessCatalog };
  get_model_catalog: { input: DTO.ModelCatalogRequest; output: DTO.ModelCatalog };
  builtin_agent_sign_in: {
    input: DTO.BuiltinAgentSignInRequest;
    output: DTO.BuiltinAgentSignInResponse;
  };
  builtin_agent_sign_in_status: {
    input: DTO.BuiltinAgentSignInStatusRequest;
    output: DTO.BuiltinAgentSignInStatus;
  };
  builtin_agent_sign_in_cancel: {
    input: DTO.BuiltinAgentSignInStatusRequest;
    output: DTO.BuiltinAgentSignInStatus;
  };
  builtin_agent_sign_out: {
    input: DTO.BuiltinAgentSignOutRequest;
    output: DTO.BuiltinAgentSignOutResponse;
  };
  get_reaction_catalog: { input: DTO.CatalogRequest; output: DTO.ReactionCatalog };
  list_provider_interfaces: {
    input: DTO.ListProviderInterfacesRequest;
    output: DTO.ListProviderInterfacesResponse;
  };
  invoke_provider_interface: {
    input: DTO.InvokeProviderInterfaceRequest;
    output: DTO.InvokeProviderInterfaceResponse;
  };
  enable_remote_access: { input: null; output: DTO.RemoteAccess };
  get_remote_access: { input: null; output: DTO.RemoteAccess };
  set_remote_access: { input: DTO.SetRemoteAccessRequest; output: DTO.RemoteAccess };
}
export async function call<K extends keyof Operations>(
  operation: K,
  input: Operations[K]['input'],
  signal?: AbortSignal,
): Promise<Operations[K]['output']> {
  let response: Response;
  try {
    response = await fetch(`/api/v1/${operation}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
      signal,
    });
  } catch (error) {
    if (signal?.aborted) throw new ApiError('cancelled', 'Request cancelled');
    throw new ApiError('unavailable', error instanceof Error ? error.message : 'Request failed');
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch {
    throw new ApiError('data_loss', 'Invalid response');
  }
  if (!response.ok) {
    const error = body as DTO.ErrorBody;
    throw new ApiError(error.code, error.message, error.detail);
  }
  return body as Operations[K]['output'];
}
