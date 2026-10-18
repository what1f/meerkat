use super::{feishu_web, telegram_web};
use feishu_web::FeishuWeb;
use lankide_client::{ManagementClient, WorkspaceDraft};
use lankide_provider::{Provider, ProviderHost};
use lankide_runtime::ProviderFactoryRegistry;
use lankide_system_e2e::{Result, SystemRig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImProvider {
    #[default]
    SystemIm,
    Feishu,
    Telegram,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scan {
    Waiting,
    Scanned,
    Expired,
    Complete,
}
#[derive(Clone, Default)]
pub struct Im {
    feishu: Arc<Mutex<BTreeMap<String, FeishuWeb>>>,
    telegram: Arc<Mutex<BTreeMap<String, Arc<telegram_web::TelegramWeb>>>>,
    sequence: Arc<AtomicU64>,
}
#[derive(Clone)]
struct TelegramNetwork(Arc<telegram_web::TelegramWeb>);
impl lankide_provider_telegram::Transport for TelegramNetwork {
    fn execute(
        &self,
        mut request: lankide_provider_telegram::Request,
    ) -> lankide_provider_telegram::http::TransportFuture<'_> {
        Box::pin(async move {
            let path = request
                .path
                .strip_prefix("https://api.telegram.org")
                .ok_or_else(|| {
                    lankide_provider_telegram::TransportError::Failed(
                        "unexpected Telegram host".into(),
                    )
                })?;
            request.path = format!("{}{}", self.0.http.endpoint, path);
            lankide_provider_telegram::Transport::execute(
                &lankide_provider_telegram::ReqwestTransport::new(),
                request,
            )
            .await
        })
    }
}
impl Im {
    fn feishu(&self, workspace: &str) -> FeishuWeb {
        self.feishu
            .lock()
            .unwrap()
            .entry(workspace.into())
            .or_insert_with(|| {
                let web = FeishuWeb::default();
                {
                    let mut state = web.0.lock().unwrap();
                    state.name = "Alice".into();
                    state.user = "lead".into();
                    state.tenant = "tenant-a".into();
                }
                web
            })
            .clone()
    }
    fn telegram(&self, workspace: &str) -> Arc<telegram_web::TelegramWeb> {
        self.telegram
            .lock()
            .unwrap()
            .entry(workspace.into())
            .or_insert_with(|| Arc::new(telegram_web::TelegramWeb::start()))
            .clone()
    }
    pub fn factories(&self) -> Result<ProviderFactoryRegistry> {
        let factories = ProviderFactoryRegistry::default();
        let im = self.clone();
        factories.register_host_factory(
            lankide_provider_feishu::FeishuProvider::new().info(),
            move |w, s| {
                let transport = im.feishu(w.id.as_str());
                ProviderHost::new(
                    lankide_provider_feishu::FeishuProvider::new()
                        .with_transport(transport.clone())
                        .with_event_source_factory(move |_, _, _, _| {
                            let source = feishu_web::Source(transport.clone());
                            Box::pin(async move {
                                Ok(Box::new(source)
                                    as Box<dyn lankide_provider_feishu::events::EventSource>)
                            })
                        }),
                    w.id.clone(),
                    w.provider_config.clone(),
                    s,
                )
            },
        )?;
        let im = self.clone();
        factories.register_host_factory(
            lankide_provider_telegram::TelegramProvider::new().info(),
            move |w, s| {
                ProviderHost::new(
                    lankide_provider_telegram::TelegramProvider::new()
                        .with_transport(TelegramNetwork(im.telegram(w.id.as_str()))),
                    w.id.clone(),
                    w.provider_config.clone(),
                    s,
                )
            },
        )?;
        Ok(factories)
    }
    pub async fn create_workspace(
        &self,
        rig: &SystemRig,
        workspace: &str,
        provider: ImProvider,
    ) -> Result<()> {
        let (id, config, secrets) = match provider {
            ImProvider::SystemIm => {
                rig.workspace(workspace).await?;
                return Ok(());
            }
            ImProvider::Feishu => (
                "feishu",
                json!({"domain":"feishu","task_system_enabled":false}),
                json!({}),
            ),
            ImProvider::Telegram => (
                "telegram",
                json!({"poll_timeout_seconds":1}),
                json!({"manager_bot_token":"100:manager"}),
            ),
        };
        rig.management()
            .create_workspace(WorkspaceDraft {
                id: workspace.into(),
                name: workspace.into(),
                rules: String::new(),
                provider_id: id.into(),
                provider_config: config,
                provider_secrets: secrets,
            })
            .await?;
        Ok(())
    }
    pub fn scan(&self, workspace: &str, scan: Scan) {
        let web = self.feishu(workspace);
        let mut state = web.0.lock().unwrap();
        state.qr_expired = matches!(scan, Scan::Expired);
        state.login_stage = match scan {
            Scan::Waiting | Scan::Expired => 0,
            Scan::Scanned => 1,
            Scan::Complete => 2,
        };
    }
    pub async fn dm(
        &self,
        rig: &SystemRig,
        workspace: &str,
        agent: &str,
        text: &str,
    ) -> Result<()> {
        let provider = rig
            .management()
            .list_workspaces()
            .await?
            .into_iter()
            .find(|w| w.id == workspace)
            .ok_or("workspace")?
            .provider
            .id;
        let before = rig
            .existing_workspace(workspace)
            .existing_agent(agent)
            .inbox_count()?;
        let id = self.sequence.fetch_add(1, Ordering::SeqCst) + 100;
        match provider.as_str() {
            "system_im" => {
                let team = rig.existing_workspace(workspace);
                // This step already acknowledges delivery (or offline IM acceptance).
                return team
                    .direct_message(&team.existing_agent(agent), "owner", text)
                    .await;
            }
            "feishu" => {
                let member = rig
                    .existing_workspace(workspace)
                    .existing_agent(agent)
                    .view()
                    .await?;
                let config: Value = member.provider_agent_config;
                let app = config["app_id"].as_str().ok_or("Feishu app_id")?;
                let web = self.feishu(workspace);
                let mut state = web.0.lock().unwrap();
                let message = format!("web-dm-{id}");
                state
                    .chat_types
                    .insert("owner_dm".into(), "private_chat".into());
                state
                    .message_chats
                    .insert(message.clone(), "owner_dm".into());
                state.events.push_back(lankide_provider_feishu::events::RawEvent::new("im.message.receive_v1",json!({"schema":"2.0","header":{"event_id":format!("web-{id}"),"event_type":"im.message.receive_v1","app_id":app,"tenant_key":"tenant-a"},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_lead","union_id":"on_lead"}},"message":{"message_id":message,"chat_id":"owner_dm","chat_type":"p2p","create_time":"1900000000000","message_type":"text","content":json!({"text":text}).to_string()}}}).to_string().into_bytes()));
            }
            "telegram" => {
                self.telegram(workspace).state.lock().unwrap().updates.push_back(json!({"update_id":id,"message":{"message_id":id,"date":1900000000,"chat":{"id":300,"type":"private"},"from":{"id":300,"first_name":"Alice"},"text":text}}));
            }
            _ => return Err("unsupported rig IM".into()),
        }
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                if rig
                    .existing_workspace(workspace)
                    .existing_agent(agent)
                    .inbox_count()?
                    > before
                {
                    break Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await??;
        Ok(())
    }
    pub fn outbound(&self) -> Vec<Value> {
        let mut values = vec![];
        for (workspace, web) in self.feishu.lock().unwrap().iter() {
            for message in &web.0.lock().unwrap().sends {
                values.push(json!({"workspace":workspace,"account":"feishu","chat":message["chat"],"channel_type":message["channel_type"],"content": message["content"]}));
            }
        }
        for (workspace, web) in self.telegram.lock().unwrap().iter() {
            for message in &web.state.lock().unwrap().sends {
                let private = message["chat_id"]
                    .as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .or_else(|| message["chat_id"].as_i64())
                    .is_some_and(|id| id > 0);
                values.push(json!({"workspace":workspace,"account":"telegram","chat":message["chat_id"].as_str().map(str::to_owned).unwrap_or_else(||message["chat_id"].to_string()),"channel_type":if private{"private_chat"}else{"group_chat"},"content":message["text"]}));
            }
        }
        values
    }
}
