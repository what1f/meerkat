//! Attachment ingress and local inspection through the system business rig.
use super::*;
use meerkat_provider::ResourceUpload;

impl SystemRig {
    pub async fn route_machine_through(&mut self, url: &str) -> Result<()> {
        std::fs::write(self.paths.run.join("machine-link.url"), url)?;
        self.restart_machine().await
    }
}

impl Workspace<'_> {
    pub async fn save_image(&self, bytes: Vec<u8>) -> Result<meerkat_core::ResourceRef> {
        self.save_attachment(ResourceUpload {
            file_name: "photo.png".into(),
            media_type: "image/png".into(),
            bytes,
        })
        .await
    }

    pub async fn save_attachment(
        &self,
        upload: ResourceUpload,
    ) -> Result<meerkat_core::ResourceRef> {
        let context = self
            .rig
            .im
            .resource_contexts
            .lock()
            .expect("resource contexts")
            .get(&self.id)
            .expect("Provider initialized")
            .clone();
        Ok(context.save_resource(upload).await?)
    }

    pub async fn image_message(
        &self,
        agent: &Agent<'_>,
        chat: &str,
        bytes: Vec<u8>,
    ) -> Result<meerkat_core::ResourceRef> {
        self.captioned_image_message(agent, chat, "", bytes).await
    }

    pub async fn captioned_image_message(
        &self,
        agent: &Agent<'_>,
        chat: &str,
        caption: &str,
        bytes: Vec<u8>,
    ) -> Result<meerkat_core::ResourceRef> {
        let context = self
            .rig
            .im
            .resource_contexts
            .lock()
            .expect("resource contexts")
            .get(&self.id)
            .expect("Provider initialized")
            .clone();
        let resource = self.save_image(bytes).await?;
        let id = self.rig.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        context
            .event_sink()
            .emit(ProviderEvent {
                receiving_agent_id: None,
                channel_id: ChannelId::new("group_chat")?,
                provider_chat_id: chat.into(),
                sender: EventSender {
                    id: "alice".into(),
                    name: "alice".into(),
                    kind: EventSenderType::Human,
                },
                kind: ProviderEventKind::Message(MessageEvent {
                    provider_message_id: format!("incoming-{id}"),
                    content: caption.into(),
                    reply_to_provider_message_id: None,
                    mentioned_agent_ids: vec![AgentId::new(&agent.id)?],
                    attachments: vec![resource.clone()],
                }),
                provider_data: json!({"event_id":format!("event-{id}"), "chat_name":chat}),
            })
            .await?;
        Ok(resource)
    }
}

impl Agent<'_> {
    pub fn resource_directory(&self) -> Result<PathBuf> {
        Ok(meerkat_machine_core::agent_directory(
            &self.rig.machine_home.path().join("workspaces"),
            &self.workspace,
            &self.id,
        )?
        .join("resources"))
    }
}
