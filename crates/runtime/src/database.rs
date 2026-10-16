use super::*;
use lankide_storage::*;
use serde_json::Value;
use std::sync::Weak;
use tokio::sync::Notify;

// Runtime's database boundary. The closure owns its inputs and connection work
// runs only on Tokio's blocking pool, including when its caller is cancelled.
#[derive(Clone)]
pub(crate) struct Database {
    // Field order matters: release SQLite before notifying the close waiter.
    storage: Arc<Storage>,
    signals: ReleaseSignal,
}

#[derive(Clone)]
struct ReleaseSignal(Arc<Signals>);

impl std::ops::Deref for ReleaseSignal {
    type Target = Arc<Signals>;
    fn deref(&self) -> &Arc<Signals> {
        &self.0
    }
}

struct Signals {
    released: Notify,
    changed: Arc<Notify>,
}

pub(crate) struct WeakDatabase {
    storage: Weak<Storage>,
    signals: Arc<Signals>,
}

impl Drop for ReleaseSignal {
    fn drop(&mut self) {
        // Release storage before signalling the shutdown waiter.
        self.0.released.notify_one();
    }
}

impl Database {
    pub(crate) fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            signals: ReleaseSignal(Arc::new(Signals {
                released: Notify::new(),
                changed: Arc::new(Notify::new()),
            })),
        }
    }
    pub(crate) fn as_ref(&self) -> &Storage {
        &self.storage
    }
    pub(crate) fn downgrade(&self) -> WeakDatabase {
        WeakDatabase {
            storage: Arc::downgrade(&self.storage),
            signals: self.signals.0.clone(),
        }
    }
    pub(crate) fn changes(&self) -> Arc<Notify> {
        self.signals.changed.clone()
    }
    pub(crate) fn wake(&self) {
        self.signals.changed.notify_one();
    }
    async fn call<T: Send + 'static>(
        &self,
        write: bool,
        operation: impl FnOnce(&Storage) -> lankide_storage::Result<T> + Send + 'static,
    ) -> lankide_storage::Result<T> {
        let database = self.clone();
        tokio::task::spawn_blocking(move || {
            let result = operation(&database.storage);
            if write {
                database.wake();
            }
            result
        })
        .await
        .map_err(|error| StorageError::Invalid(format!("database task failed: {error}")))?
    }
    pub(crate) async fn close(
        self,
        timeout: std::time::Duration,
    ) -> std::result::Result<(), RuntimeError> {
        let signals = self.signals.clone();
        let mut storage = self.storage.clone();
        drop(self);
        tokio::time::timeout(timeout, async {
            loop {
                // Register before inspecting ownership; a release between these
                // operations leaves a permit and cannot be missed.
                let released = signals.released.notified();
                match Arc::try_unwrap(storage) {
                    Ok(storage) => {
                        return tokio::task::spawn_blocking(move || storage.close())
                            .await
                            .map_err(|error| RuntimeError::Application(error.to_string()))?
                            .map_err(RuntimeError::from);
                    }
                    Err(shared) => storage = shared,
                }
                released.await;
            }
        })
        .await
        .map_err(|_| RuntimeError::StorageStillShared)?
    }
}

impl WeakDatabase {
    pub(crate) fn upgrade(&self) -> Option<Database> {
        Some(Database {
            storage: self.storage.upgrade()?,
            signals: ReleaseSignal(self.signals.clone()),
        })
    }
    pub(crate) fn wake(&self) {
        self.signals.changed.notify_one();
    }
}

macro_rules! database_methods {
    ($($write:literal $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $result:ty [$($prepare:tt)*] ($($input:expr),* $(,)?);)*) => {
        impl Database {
            $(pub(crate) async fn $name(&self, $($arg: $ty),*) -> lankide_storage::Result<$result> {
                $($prepare)*
                self.call($write, move |storage| storage.$name($($input),*)).await
            })*
        }
    };
}

database_methods! {
    false next_agent_work_at(
        workspace: &WorkspaceId,
        agent: &AgentId,
        dispatchable: bool,
    ) -> Option<i64> [
        let workspace = workspace.to_owned();
        let agent = agent.to_owned();
    ] (&workspace, &agent, dispatchable);
    false next_attempt_lease_at() -> Option<i64> [] ();
    true accept_execution_attempt(
        execution_id: i64,
        attempt: i64,
        accepted: bool,
        now: i64,
    ) -> bool [
    ] (execution_id, attempt, accepted, now);
    false active_execution_ids_for_agent(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Vec<i64> [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
    ] (&workspace_id, &agent_id);
    false agent_pause_reason(
        w: &WorkspaceId,
        a: &AgentId,
    ) -> Option<String> [
        let w = w.to_owned();
        let a = a.to_owned();
    ] (&w, &a);
    false agent_provider_state(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Value [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
    ] (&workspace_id, &agent_id);
    false agent_record(
        w: &WorkspaceId,
        a: &AgentId,
    ) -> AgentRecord [
        let w = w.to_owned();
        let a = a.to_owned();
    ] (&w, &a);
    true allocate_resource_id(
        workspace_id: &WorkspaceId,
        now: i64,
    ) -> ResourceId [
        let workspace_id = workspace_id.to_owned();
    ] (&workspace_id, now);
    true apply_failure_policy(
        id: i64,
        fingerprint: &str,
        pause_reason: Option<&str>,
        retry_at: Option<i64>,
        notice: bool,
        dedupe_ms: i64,
        now: i64,
    ) -> () [
        let fingerprint = fingerprint.to_owned();
        let pause_reason = pause_reason.map(ToOwned::to_owned);
    ] (id, &fingerprint, pause_reason.as_deref(), retry_at, notice, dedupe_ms, now);
    false attempt_authorization(
        execution_id: i64,
        attempt: i64,
    ) -> Option<AttemptAuthorization> [
    ] (execution_id, attempt);
    true begin_send(r: SendRequest) -> SendAttemptView [] (r);
    true begin_send_guarded(
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        force: bool,
        r: SendRequest,
    ) -> GuardedSendResult [
        let channel_id = channel_id.to_owned();
        let provider_chat_id = provider_chat_id.to_owned();
    ] (snapshot_id, &channel_id, &provider_chat_id, force, r);
    true begin_tool_invocation(r: ToolInvocationRequest) -> ToolInvocationResult [] (r);
    true cancel_execution(id: i64, now: i64) -> bool [] (id, now);
    false chat_attention(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        provider_chat_id: &str,
    ) -> Option<AttentionRecord> [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
        let channel_id = channel_id.to_owned();
        let provider_chat_id = provider_chat_id.to_owned();
    ] (&workspace_id, &agent_id, &channel_id, &provider_chat_id);
    true commit_execution(
        id: i64,
        out: &str,
        response_send_request_key: Option<&str>,
        now: i64,
    ) -> CommitResult [
        let out = out.to_owned();
        let response_send_request_key = response_send_request_key.map(ToOwned::to_owned);
    ] (id, &out, response_send_request_key.as_deref(), now);
    true compare_and_set_person_state(
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        expected_revision: i64,
        provider_identity_state: &Value,
        now: i64,
    ) -> PersonRecord [
        let workspace_id = workspace_id.to_owned();
        let person_id = person_id.to_owned();
        let provider_identity_state = provider_identity_state.to_owned();
    ] (&workspace_id, &person_id, expected_revision, &provider_identity_state, now);
    true complete_agent_file_deletion(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> bool [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
    ] (&workspace_id, &agent_id);
    true create_execution_with_auth(
        w: &WorkspaceId,
        a: &AgentId,
        summary: &str,
        now: i64,
        auth: ExecutionAuthSpec,
    ) -> Option<CreatedExecution> [
        let w = w.to_owned();
        let a = a.to_owned();
        let summary = summary.to_owned();
    ] (&w, &a, &summary, now, auth);
    true delete_agent(w: &WorkspaceId, a: &AgentId) -> bool [let w = w.to_owned(); let a = a.to_owned();] (&w, &a);
    false execution(id: i64) -> ExecutionView [] (id);
    false execution_attempt(
        execution_id: i64,
        attempt: i64,
    ) -> Option<ExecutionAttemptView> [
    ] (execution_id, attempt);
    false execution_attempts(execution_id: i64) -> Vec<ExecutionAttemptView> [] (execution_id);
    false execution_auth_context(
        execution_id: i64,
        attempt: i64,
    ) -> Option<ExecutionAuthContext> [
    ] (execution_id, attempt);
    false execution_machine_matches(
        execution_id: i64,
        attempt: i64,
        machine_id: &str,
    ) -> bool [
        let machine_id = machine_id.to_owned();
    ] (execution_id, attempt, &machine_id);
    false execution_placement_matches(
        execution_id: i64,
        attempt: i64,
        machine_id: &str,
    ) -> bool [
        let machine_id = machine_id.to_owned();
    ] (execution_id, attempt, &machine_id);
    true expire_execution_attempt_leases(now: i64) -> Vec<(i64, i64)> [] (now);
    true fail_execution(id: i64, e: &str, now: i64) -> bool [let e = e.to_owned();] (id, &e, now);
    true fail_execution_retry_after(
        id: i64,
        error: &str,
        detail: &str,
        retry_at: i64,
        now: i64,
    ) -> bool [
        let error = error.to_owned();
        let detail = detail.to_owned();
    ] (id, &error, &detail, retry_at, now);
    true fail_tool_invocation(
        request: &ToolInvocationRequest,
        code: &str,
        now: i64,
    ) -> bool [
        let request = request.to_owned();
        let code = code.to_owned();
    ] (&request, &code, now);
    false failure_history(id: i64) -> FailureHistory [] (id);
    true finish_failure_notice(id: i64, now: i64) -> () [] (id, now);
    true begin_execution_send(execution_id: i64, request: SendRequest) -> SendAttemptView [] (execution_id,request);
    true bind_sent_chat(message_id: i64, native_chat: &str) -> () [let native_chat=native_chat.to_owned();] (message_id,&native_chat);
    true prepare_direct_chat(workspace: &WorkspaceId, agent: &AgentId, channel: &ChannelId, channel_name: &str, person: &PersonId, now: i64) -> String [let workspace=workspace.clone(); let agent=agent.clone(); let channel=channel.clone(); let channel_name=channel_name.to_owned(); let person=person.clone();] (&workspace,&agent,&channel,&channel_name,&person,now);
    true record_outbound_resources(message_id: i64, resources: Vec<ResourceView>) -> () [] (message_id,resources);
    true finish_message_send(
        message_id: i64, outcome: SendOutcome, provider_message_ids: Vec<String>,
        resources: Vec<ResourceView>, now: i64,
    ) -> FinishSendResult [] (message_id, outcome, provider_message_ids, resources, now);
    true finish_send(
        request_key: &str,
        outcome: SendOutcome,
        now: i64,
    ) -> FinishSendResult [
        let request_key = request_key.to_owned();
    ] (&request_key, outcome, now);
    true finish_tool_invocation(
        r: &ToolInvocationRequest,
        v: Value,
        now: i64,
    ) -> bool [
        let r = r.to_owned();
    ] (&r, v, now);
    true freeze_attempt_authorization(authorization: AttemptAuthorization) -> () [] (authorization);
    true get_or_create_person(
        workspace_id: &WorkspaceId,
        provider_instance_id: &ProviderId,
        external_id: &str,
        now: i64,
    ) -> PersonRecord [
        let workspace_id = workspace_id.to_owned();
        let provider_instance_id = provider_instance_id.to_owned();
        let external_id = external_id.to_owned();
    ] (&workspace_id, &provider_instance_id, &external_id, now);
    false has_active_execution(
        workspace_id: &WorkspaceId,
    ) -> bool [
        let workspace_id = workspace_id.to_owned();
    ] (&workspace_id);
    true ingest_event(event: IngestEvent) -> IngestResult [] (event);
    true initialize_team_lead(
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        now: i64,
    ) -> PersonRecord [
        let workspace_id = workspace_id.to_owned();
        let person_id = person_id.to_owned();
    ] (&workspace_id, &person_id, now);
    true insert_new_agent_record(
        record: AgentRecord,
        provider_secrets: &Value,
    ) -> AgentRecord [
        let provider_secrets = provider_secrets.to_owned();
    ] (record, &provider_secrets);
    true insert_new_workspace_record(
        record: WorkspaceRecord,
        provider_secrets: &Value,
    ) -> WorkspaceRecord [
        let provider_secrets = provider_secrets.to_owned();
    ] (record, &provider_secrets);
    true issue_pairing_code(
        code_hash: &str,
        issued_at: i64,
    ) -> () [
        let code_hash = code_hash.to_owned();
    ] (&code_hash, issued_at);
    false list_agent_records(w: &WorkspaceId) -> Vec<AgentRecord> [let w = w.to_owned();] (&w);
    false list_agents(w: &WorkspaceId) -> Vec<AgentView> [let w = w.to_owned();] (&w);
    false list_human_team_member_views(
        workspace_id: &WorkspaceId,
    ) -> Vec<HumanTeamMemberView> [
        let workspace_id = workspace_id.to_owned();
    ] (&workspace_id);
    false list_human_team_members(
        workspace_id: &WorkspaceId,
    ) -> Vec<PersonRecord> [
        let workspace_id = workspace_id.to_owned();
    ] (&workspace_id);
    false list_workspace_records() -> Vec<WorkspaceRecord> [] ();
    true mark_attempt_delivery(
        execution_id: i64,
        attempt: i64,
        delivery_id: i64,
        accepted: bool,
        now: i64,
    ) -> bool [
    ] (execution_id, attempt, delivery_id, accepted, now);
    true mark_delivery_delivered(id: i64, now: i64) -> bool [] (id, now);
    true mark_execution_recovering(id: i64, now: i64) -> bool [] (id, now);
    true mark_execution_running(id: i64, now: i64) -> bool [] (id, now);
    true mark_steer_delivery_result(
        delivery_id: i64,
        execution_id: i64,
        attempt: i64,
        machine_id: &str,
        machine_instance_id: &str,
        accepted: bool,
        now: i64,
    ) -> bool [
        let machine_id = machine_id.to_owned();
        let machine_instance_id = machine_instance_id.to_owned();
    ] (delivery_id, execution_id, attempt, &machine_id, &machine_instance_id, accepted, now);
    true observe_machine(
        machine_id: &str,
        now: i64,
        inventory_json: Option<&str>,
        os: Option<&str>,
        version: Option<&str>,
    ) -> () [
        let machine_id = machine_id.to_owned();
        let inventory_json = inventory_json.map(ToOwned::to_owned);
        let os = os.map(ToOwned::to_owned);
        let version = version.map(ToOwned::to_owned);
    ] (&machine_id, now, inventory_json.as_deref(), os.as_deref(), version.as_deref());
    true offer_execution_attempt(
        execution_id: i64,
        attempt: i64,
        machine_id: &str,
        machine_instance_id: &str,
        lease_expires_at: i64,
        now: i64,
    ) -> ExecutionAttemptView [
        let machine_id = machine_id.to_owned();
        let machine_instance_id = machine_instance_id.to_owned();
    ] (execution_id, attempt, &machine_id, &machine_instance_id, lease_expires_at, now);
    true pair_machine(
        machine: &MachineRecord,
        code_hash: Option<&str>,
        validity_ms: i64,
    ) -> () [
        let machine = machine.to_owned();
        let code_hash = code_hash.map(ToOwned::to_owned);
    ] (&machine, code_hash.as_deref(), validity_ms);
    false pending_agent_file_deletions(
        machine_id: &str,
    ) -> Vec<(WorkspaceId, AgentId)> [
        let machine_id = machine_id.to_owned();
    ] (&machine_id);
    false pending_failure_notices(
        w: &WorkspaceId,
        a: &AgentId,
    ) -> Vec<FailureNotice> [
        let w = w.to_owned();
        let a = a.to_owned();
    ] (&w, &a);
    false pending_steer_deliveries(
        machine_id: &str,
        machine_instance_id: &str,
    ) -> Vec<SteerDelivery> [
        let machine_id = machine_id.to_owned();
        let machine_instance_id = machine_instance_id.to_owned();
    ] (&machine_id, &machine_instance_id);
    false person(
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
    ) -> Option<PersonRecord> [
        let workspace_id = workspace_id.to_owned();
        let person_id = person_id.to_owned();
    ] (&workspace_id, &person_id);
    false provider_kv_get(
        workspace_id: &WorkspaceId,
        namespace: &str,
        key: &str,
    ) -> Option<Vec<u8>> [
        let workspace_id = workspace_id.to_owned();
        let namespace = namespace.to_owned();
        let key = key.to_owned();
    ] (&workspace_id, &namespace, &key);
    true provider_kv_put(
        workspace_id: &WorkspaceId,
        namespace: &str,
        key: &str,
        value: &[u8],
        now: i64,
    ) -> () [
        let workspace_id = workspace_id.to_owned();
        let namespace = namespace.to_owned();
        let key = key.to_owned();
        let value = value.to_owned();
    ] (&workspace_id, &namespace, &key, &value, now);
    false provider_message(
        workspace_id: &WorkspaceId,
        message_id: i64,
    ) -> Option<ProviderMessageView> [
        let workspace_id = workspace_id.to_owned();
    ] (&workspace_id, message_id);
    true put_agent_provider_state(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        state: &Value,
    ) -> () [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
        let state = state.to_owned();
    ] (&workspace_id, &agent_id, &state);
    true put_agent_secret(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        name: &str,
        value: SecretValue,
        now: i64,
    ) -> () [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
        let name = name.to_owned();
    ] (&workspace_id, &agent_id, &name, value, now);
    true put_secret(
        workspace_id: &WorkspaceId,
        name: &str,
        value: SecretValue,
        now: i64,
    ) -> WorkspaceSecretRef [
        let workspace_id = workspace_id.to_owned();
        let name = name.to_owned();
    ] (&workspace_id, &name, value, now);
    true put_workspace_provider_state(
        workspace_id: &WorkspaceId,
        state: &Value,
    ) -> () [
        let workspace_id = workspace_id.to_owned();
        let state = state.to_owned();
    ] (&workspace_id, &state);
    false reaction_target_author(
        workspace: &WorkspaceId,
        channel: &ChannelId,
        chat: &str,
        target: &str,
        receiver: Option<&AgentId>,
    ) -> Option<(MessageAuthorType, String)> [
        let workspace = workspace.to_owned();
        let channel = channel.to_owned();
        let chat = chat.to_owned();
        let target = target.to_owned();
        let receiver = receiver.map(ToOwned::to_owned);
    ] (&workspace, &channel, &chat, &target, receiver.as_ref());
    false read_secret(
        workspace_id: &WorkspaceId,
        name: &str,
    ) -> Option<SecretValue> [
        let workspace_id = workspace_id.to_owned();
        let name = name.to_owned();
    ] (&workspace_id, &name);
    false recall_target(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        message_id: i64,
    ) -> Option<RecallTarget> [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
    ] (&workspace_id, &agent_id, message_id);
    true reconcile_machine_heartbeat(
        machine_id: &str,
        instance_id: &str,
        active_attempts: &[(i64, i64)],
        now: i64,
        lease_expires_at: i64,
    ) -> HeartbeatReconciliation [
        let machine_id = machine_id.to_owned();
        let instance_id = instance_id.to_owned();
        let active_attempts = active_attempts.to_owned();
    ] (&machine_id, &instance_id, &active_attempts, now, lease_expires_at);
    true record_execution_event(event: RecordExecutionEvent) -> ExecutionEventReceipt [] (event);
    true record_reaction(reaction: IncomingReaction) -> ReactionResult [] (reaction);
    true recover_startup(now: i64) -> RecoveryState [] (now);
    false recoverable_executions(
        workspace_id: Option<&WorkspaceId>,
    ) -> Vec<RecoverableExecutionView> [
        let workspace_id = workspace_id.map(ToOwned::to_owned);
    ] (workspace_id.as_ref());
    false recoverable_sends(
        workspace_id: Option<&WorkspaceId>,
    ) -> Vec<RecoverableSendView> [
        let workspace_id = workspace_id.map(ToOwned::to_owned);
    ] (workspace_id.as_ref());
    false registered_machines() -> Vec<MachineRecord> [] ();
    true reject_execution_attempt_with_failure(
        execution_id: i64,
        attempt: i64,
        error_code: &str,
        error_message: &str,
        failure: Option<&Value>,
        now: i64,
    ) -> bool [
        let error_code = error_code.to_owned();
        let error_message = error_message.to_owned();
        let failure = failure.map(ToOwned::to_owned);
    ] (execution_id, attempt, &error_code, &error_message, failure.as_ref(), now);
    false resource_metadata(
        w: &WorkspaceId,
        id: ResourceId,
    ) -> Option<ResourceMetadata> [
        let w = w.to_owned();
    ] (&w, id);
    true resume_agent(
        w: &WorkspaceId,
        a: &AgentId,
        now: i64,
    ) -> () [
        let w = w.to_owned();
        let a = a.to_owned();
    ] (&w, &a, now);
    true retry_execution_conflict(id: i64, retry_at: i64) -> () [] (id, retry_at);
    true rename_machine(machine_id: &str, name: &str) -> () [
        let machine_id = machine_id.to_owned();
        let name = name.to_owned();
    ] (&machine_id, &name);
    true revoke_machine(
        machine_id: &str,
        now: i64,
    ) -> () [
        let machine_id = machine_id.to_owned();
    ] (&machine_id, now);
    true rollback_new_agent(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> () [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
    ] (&workspace_id, &agent_id);
    true rollback_new_workspace(id: &WorkspaceId) -> () [let id = id.to_owned();] (&id);
    true set_chat_attention(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        attention: &AttentionRecord,
        now: i64,
    ) -> () [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
        let channel_id = channel_id.to_owned();
        let provider_chat_id = provider_chat_id.to_owned();
        let attention = attention.to_owned();
    ] (&workspace_id, &agent_id, &channel_id, &provider_chat_id, &attention, now);
    true set_workspace_active(
        workspace_id: &WorkspaceId,
        active: bool,
        now: i64,
    ) -> bool [
        let workspace_id = workspace_id.to_owned();
    ] (&workspace_id, active, now);
    true set_workspace_setup(
        workspace_id: &WorkspaceId,
        status: WorkspaceSetupStatus,
        error_code: Option<&str>,
        now: i64,
    ) -> WorkspaceRecord [
        let workspace_id = workspace_id.to_owned();
        let error_code = error_code.map(ToOwned::to_owned);
    ] (&workspace_id, status, error_code.as_deref(), now);
    false snapshot(snapshot_id: i64) -> SnapshotView [] (snapshot_id);
    true record_chat_receiver(workspace: &WorkspaceId, agent: &AgentId, channel: &ChannelId, chat: &str) -> () [
        let workspace=workspace.clone(); let agent=agent.clone(); let channel=channel.clone(); let chat=chat.to_owned();
    ] (&workspace, &agent, &channel, &chat);
    false agent_can_access_chat(workspace: &WorkspaceId, agent: &AgentId, channel: &ChannelId, chat: &str) -> bool [
        let workspace=workspace.clone(); let agent=agent.clone(); let channel=channel.clone(); let chat=chat.to_owned();
    ] (&workspace, &agent, &channel, &chat);
    true snapshot_chat_messages(
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        before_message_id: Option<i64>,
        limit: usize,
        now: i64,
    ) -> Vec<MessageView> [
        let channel_id = channel_id.to_owned();
        let provider_chat_id = provider_chat_id.to_owned();
    ] (snapshot_id, &channel_id, &provider_chat_id, before_message_id, limit, now);
    false snapshot_contains_message(
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        message_id: i64,
    ) -> bool [
        let channel_id = channel_id.to_owned();
        let provider_chat_id = provider_chat_id.to_owned();
    ] (snapshot_id, &channel_id, &provider_chat_id, message_id);
    false snapshot_message(snapshot_id: i64, message_id: i64) -> Option<MessageView> [] (snapshot_id, message_id);
    false unapplied_failures(
        w: &WorkspaceId,
        a: &AgentId,
    ) -> Vec<(i64, i64)> [
        let w = w.to_owned();
        let a = a.to_owned();
    ] (&w, &a);
    true update_agent_record_with_secrets(
        record: AgentRecord,
        provider_secrets: &Value,
    ) -> AgentRecord [
        let provider_secrets = provider_secrets.to_owned();
    ] (record, &provider_secrets);
    true update_outbound_message_content(
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        message_id: i64,
        content: &str,
        now: i64,
    ) -> bool [
        let workspace_id = workspace_id.to_owned();
        let agent_id = agent_id.to_owned();
        let content = content.to_owned();
    ] (&workspace_id, &agent_id, message_id, &content, now);
    true update_team_member(
        workspace_id: &WorkspaceId,
        actor: &PersonId,
        target: &PersonId,
        role: TeamRole,
        now: i64,
    ) -> PersonRecord [
        let workspace_id = workspace_id.to_owned();
        let actor = actor.to_owned();
        let target = target.to_owned();
    ] (&workspace_id, &actor, &target, role, now);
    true update_workspace_record_with_secrets(
        record: WorkspaceRecord,
        provider_secrets: &Value,
    ) -> WorkspaceRecord [
        let provider_secrets = provider_secrets.to_owned();
    ] (record, &provider_secrets);
    true upsert_resource_metadata(metadata: ResourceMetadata) -> ResourceMetadata [] (metadata);
    false workspace(id: &WorkspaceId) -> WorkspaceView [let id = id.to_owned();] (&id);
    false workspace_provider_state(
        workspace_id: &WorkspaceId,
    ) -> Value [
        let workspace_id = workspace_id.to_owned();
    ] (&workspace_id);
    false workspace_record(id: &WorkspaceId) -> WorkspaceRecord [let id = id.to_owned();] (&id);
}
