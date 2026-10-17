//! Development-only management contract exporter.
use lankide_protocol::management::*;
use ts_rs::TS;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("expected output directory")?;
    let output = std::path::PathBuf::from(output);
    std::fs::create_dir_all(&output)?;
    // This dedicated directory contains only generated TypeScript bindings.
    // Remove obsolete types as well as replacing changed declarations.
    for entry in std::fs::read_dir(&output)? {
        let path = entry?.path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "ts") {
            std::fs::remove_file(path)?;
        }
    }
    let config = ts_rs::Config::default()
        .with_large_int("number")
        .with_out_dir(&output);
    let mut names = Vec::new();
    macro_rules! export {
        ($($ty:ty),* $(,)?) => {$({
            <$ty>::export_all(&config)?;
            names.push(<$ty>::ident(&config));
        })*};
    }
    export!(
        AdvanceAgentOnboardingRequest,
        AdvanceProviderOnboardingRequest,
        Agent,
        AgentAttention,
        AgentAvatarFile,
        AgentAvatarUpload,
        AgentOnboardingState,
        AgentRef,
        AttemptFailure,
        AttentionMode,
        CancelAttemptRequest,
        CancelAttemptResponse,
        CatalogRequest,
        ChannelAttention,
        CreateAgentRequest,
        CreateWorkspaceRequest,
        DeleteAgentRequest,
        DeleteAgentResponse,
        DesiredState,
        EnvironmentVariable,
        EnvironmentVariableSet,
        ErrorBody,
        ErrorCode,
        EventFrame,
        ExecutionAttemptDetails,
        ExecutionDetails,
        GetAgentAvatarRequest,
        GetAgentAvatarResponse,
        GetAgentOnboardingRequest,
        GetAgentRequest,
        GetExecutionDetailsRequest,
        GetProviderOnboardingRequest,
        GetWorkspaceRequest,
        HarnessAvailability,
        HarnessCatalog,
        HarnessCatalogItem,
        HarnessInfo,
        InvokeProviderInterfaceRequest,
        InvokeProviderInterfaceResponse,
        ListAgentsRequest,
        ListAgentsResponse,
        ListProviderInterfacesRequest,
        ListProviderInterfacesResponse,
        ListWorkspacesRequest,
        ListWorkspacesResponse,
        MachineCatalog,
        MachineCatalogItem,
        ManagementEvent,
        ManagementEventKind,
        ManagementSubscription,
        ModelCatalog,
        ModelCatalogItem,
        ModelCatalogRequest,
        ObservedState,
        OnboardingStatus,
        PairingCode,
        ProviderCatalog,
        ProviderCatalogItem,
        ProviderInterface,
        ProviderOnboardingState,
        ReactionCatalog,
        ReactionCatalogItem,
        RemoteAccess,
        RenameMachineRequest,
        ResourceRef,
        RevokeMachineRequest,
        SetRemoteAccessRequest,
        SetWorkspaceStateRequest,
        UpdateAgentRequest,
        UpdateWorkspaceRequest,
        UploadWorkspaceImageRequest,
        UploadWorkspaceImageResponse,
        Workspace,
        WorkspaceImageRequest,
        WorkspaceImageResponse,
        WorkspaceRef,
    );
    names.sort();
    names.dedup();
    let index = names
        .into_iter()
        .map(|name| format!("export type {{ {name} }} from './{name}';\n"))
        .collect::<String>();
    std::fs::write(output.join("index.ts"), index)?;
    Ok(())
}
