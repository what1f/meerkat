use super::*;

impl Runtime {
    pub(crate) async fn apply_pending_failure_policies(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
    ) -> Result<(), RuntimeError> {
        for (id, failed_at) in self.storage.unapplied_failures(w, a).await? {
            let attempt = self
                .storage
                .execution_attempts(id)
                .await?
                .into_iter()
                .last();
            let failure = attempt.and_then(|attempt| attempt.failure);
            let category = failure
                .as_ref()
                .and_then(|f| f.get("category"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("transient");
            if category == "conflict" {
                self.storage
                    .retry_execution_conflict(
                        id,
                        failed_at.saturating_add(self.config.retry.base_ms.max(1)),
                    )
                    .await?;
                continue;
            }
            let cancelled = self.storage.execution(id).await?.state
                == meerkat_storage::ExecutionState::Cancelled
                || category == "cancelled";
            let fingerprint = failure
                .as_ref()
                .and_then(|f| f.get("fingerprint"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unclassified_attempt_failure");
            let detail = failure
                .as_ref()
                .and_then(|f| f.get("detail"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Attempt failed");
            let history = self.storage.failure_history(id).await?;
            let streak = if history.fingerprint.as_deref() == Some(fingerprint) {
                history.streak + 1
            } else {
                1
            };
            let pause = !cancelled
                && (matches!(category, "configuration" | "authentication") || streak >= 3);
            let stop = cancelled || pause || history.batch_failures + 1 >= 5;
            let config = &self.config.retry;
            let delay = config
                .base_ms
                .max(1)
                .saturating_mul(
                    1_i64
                        .checked_shl(history.batch_failures.min(62) as u32)
                        .unwrap_or(i64::MAX),
                )
                .min(config.max_ms.max(1));
            self.storage
                .apply_failure_policy(
                    id,
                    fingerprint,
                    pause.then_some(detail),
                    (!stop).then_some(failed_at.saturating_add(delay)),
                    !cancelled && stop,
                    config.notice_dedupe_ms,
                    failed_at,
                )
                .await?;
        }
        let agent = self.storage.agent_record(w, a).await?;
        for notice in self.storage.pending_failure_notices(w, a).await? {
            let channel_id = ChannelId::new(notice.channel_id.clone())
                .map_err(|e| RuntimeError::InvalidCore(e.to_string()))?;
            let key = format!("failure-notice:{}", notice.id);
            let send = self
                .storage
                .begin_send(SendRequest {
                    workspace_id: w.clone(),
                    channel_id: channel_id.clone(),
                    provider_chat_id: notice.provider_chat_id.clone(),
                    request_key: key.clone(),
                    sender_agent_id: a.clone(),
                    content: notice.content.clone(),
                    now: now_millis(),
                })
                .await?;
            if send.state == "sent" {
                self.storage
                    .finish_failure_notice(notice.id, now_millis())
                    .await?;
                continue;
            }
            let result = self
                .send_message(
                    w,
                    a.clone(),
                    SendMessageSpec {
                        channel_id,
                        chat_id: notice.provider_chat_id,
                        message_id: MessageId::new(send.message_id)
                            .map_err(|e| RuntimeError::InvalidCore(e.to_string()))?,
                        idempotency_key: format!("failure-notice:{}", notice.id),
                        content: notice.content,
                        reply_to: None,
                        mentions: Vec::new(),
                        attachments: Vec::new(),
                        force: true,
                        provider_agent_config: agent.provider_agent_config.clone(),
                    },
                )
                .await?;
            if result.status == meerkat_provider::SendMessageStatus::Sent {
                self.storage
                    .finish_send(
                        &key,
                        SendOutcome::Sent {
                            provider_message_id: result.provider_message_id.ok_or_else(|| {
                                RuntimeError::InvalidCore("missing Provider message ID".into())
                            })?,
                        },
                        now_millis(),
                    )
                    .await?;
                self.storage
                    .finish_failure_notice(notice.id, now_millis())
                    .await?;
            }
        }
        Ok(())
    }
}
