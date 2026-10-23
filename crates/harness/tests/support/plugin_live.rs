//! Explicit, isolated acceptance for a shipped JavaScript adapter.
use lankide_harness::{
    AgentEventKind, ExecuteRequest, HarnessConfig, HarnessFactory, ModelRef, SessionRequest,
    ToolDefinition, ToolResultRequest,
};
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio_stream::StreamExt;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn run(factory: &dyn HarnessFactory, cli: PathBuf, model: &str, effort: &str) -> Result {
    assert_eq!(std::env::var("LANKIDE_E2E_LIVE").as_deref(), Ok("1"));
    let directory = tempfile::Builder::new()
        .prefix("lankide-plugin-live-")
        .tempdir()?;
    let nonce = format!(
        "nonce-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let request = SessionRequest {
        cli_version: None,
        config:HarnessConfig{harness_id:factory.info().harness_id.clone(),cli_path:cli,
            model:ModelRef::new(model,model),thinking_level:effort.into(),launch_arguments:Vec::new(),
            environment:BTreeMap::new(),config_fingerprint:nonce.clone()},
        working_directory:directory.path().to_owned(), permission_policy:Default::default(),
        system_prompt:"Follow the user's test instructions. The acceptance_echo tool is a harmless test tool. Its result is supplied by the client. Finish with the requested exact text.".into(),
        tools:vec![ToolDefinition{name:"acceptance_echo".into(),description:"Echo a test nonce.".into(),input_schema:json!({"type":"object","properties":{"nonce":{"type":"string"}},"required":["nonce"]}),output_schema:json!({"type":"object"})}],
        native_session_id:None,
    };
    let probe = factory.probe(&request.config).await?;
    assert_eq!(
        probe.availability,
        lankide_harness::HarnessAvailability::Available,
        "Native CLI probe failed: {:?}",
        probe.reason
    );
    let session = factory.start(request.clone()).await?;
    let first_id = session.entry().native_session_id;
    assert!(!first_id.is_empty());
    let mut stream=session.execute(ExecuteRequest{input:format!("Remember the nonce {nonce}. Call acceptance_echo with that nonce exactly once. After its result, reply exactly PLUGIN_OK.")}).await?;
    let mut called = false;
    let outcome = tokio::time::timeout(Duration::from_secs(150), async {
        while let Some(event) = stream.next().await {
            match event?.kind {
                AgentEventKind::ToolStarted {
                    tool_call_id,
                    tool_name,
                    arguments,
                } => {
                    assert_eq!(tool_name, "acceptance_echo");
                    assert_eq!(arguments["nonce"], nonce);
                    called = true;
                    session
                        .tool_result(ToolResultRequest {
                            tool_call_id,
                            success: true,
                            content_markdown: nonce.clone(),
                            structured_content: json!({"nonce":nonce}),
                            resources: Vec::new(),
                            error: None,
                        })
                        .await?;
                }
                AgentEventKind::Completed { final_output } => {
                    assert!(called, "Injected tool was not called");
                    assert!(final_output.contains("PLUGIN_OK"));
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                AgentEventKind::Failed { code, message, .. } => {
                    return Err(format!("{code}: {message}").into());
                }
                _ => {}
            }
        }
        Err("Live stream ended without completion".into())
    })
    .await;
    session.close().await?;
    outcome??;
    let resumed = factory
        .resume(SessionRequest {
            native_session_id: Some(first_id.clone()),
            ..request
        })
        .await?;
    assert!(!resumed.entry().native_session_id.is_empty());
    let mut stream = resumed
        .execute(ExecuteRequest {
            input:
                "Reply with the nonce you remembered in the previous turn. Do not call any tools."
                    .into(),
        })
        .await?;
    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        while let Some(event) = stream.next().await {
            match event?.kind {
                AgentEventKind::Completed { final_output } => {
                    assert!(
                        final_output.contains(&nonce),
                        "Resume lost the previous context: {final_output}"
                    );
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                AgentEventKind::Failed { code, message, .. } => {
                    return Err(format!("{code}: {message}").into());
                }
                _ => {}
            }
        }
        Err("Resumed stream ended without completion".into())
    })
    .await;
    resumed.close().await?;
    outcome??;
    println!(
        "Plugin live acceptance passed: harness={}, model={model}, thinking={effort}, tools=true, resume=true",
        factory.info().harness_id
    );
    Ok(())
}
