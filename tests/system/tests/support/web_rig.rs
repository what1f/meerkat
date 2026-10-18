#[allow(dead_code)]
mod feishu_web;
#[allow(dead_code)]
mod im_http;
#[allow(dead_code)]
mod telegram_web;
mod web_im;
mod web_machines;
use lankide_client::ManagementClient;
// Non-shipping browser rig. One JSON request/reply per loopback TCP connection.
use lankide_protocol::management::URL_FILE;
use lankide_system_e2e::{Result, RigConfig, SystemRig};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io::Write, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    CreateWorkspace {
        workspace: String,
        #[serde(default)]
        provider: web_im::ImProvider,
        #[serde(default = "yes")]
        owner_avatar: bool,
    },
    SetDuty {
        workspace: String,
        on: bool,
    },
    FeishuScan {
        workspace: String,
        state: web_im::Scan,
    },
    /// Feishu stops accepting the owner's sign-in (true) or accepts it again (false).
    FeishuAuthorization {
        workspace: String,
        revoked: bool,
    },
    CreateAgent {
        workspace: String,
        agent: String,
    },
    Mention {
        workspace: String,
        agent: String,
        chat: String,
        person: String,
        text: String,
    },
    HoldNextRun {
        workspace: String,
        agent: String,
    },
    ReleaseRun {
        workspace: String,
        agent: String,
    },
    WaitRunning {
        workspace: String,
        agent: String,
    },
    PairMachine {
        command: String,
    },
    Dm {
        workspace: String,
        agent: String,
        text: String,
    },
    Settle,
}

fn yes() -> bool {
    true
}

async fn control(
    rig: &SystemRig,
    machines: &mut web_machines::Machines,
    im: &web_im::Im,
    command: Command,
) -> Result<Value> {
    match command {
        Command::FeishuScan { workspace, state } => im.scan(&workspace, state),
        Command::FeishuAuthorization { workspace, revoked } => {
            im.revoke_authorization(&workspace, revoked)
        }
        Command::PairMachine { command } => return machines.pair(rig, &command).await,
        Command::Dm {
            workspace,
            agent,
            text,
        } => {
            im.dm(rig, &workspace, &agent, &text).await?;
        }
        Command::CreateWorkspace {
            workspace,
            provider,
            owner_avatar,
        } => {
            im.create_workspace(rig, &workspace, provider, owner_avatar)
                .await?;
        }
        Command::SetDuty { workspace, on } => {
            rig.management()
                .set_workspace_state(
                    &workspace,
                    if on {
                        lankide_client::WorkspaceStatus::Active
                    } else {
                        lankide_client::WorkspaceStatus::Paused
                    },
                )
                .await?;
        }
        Command::CreateAgent { workspace, agent } => {
            let machine = lankide_machine::read_credentials(rig.machine_home())?
                .ok_or("local Machine credentials")?;
            let provider = rig
                .management()
                .list_workspaces()
                .await?
                .into_iter()
                .find(|w| w.id == workspace)
                .ok_or("workspace")?
                .provider
                .id;
            let team = rig.existing_workspace(&workspace);
            let mut builder = team.agent(&agent).machine(&machine.machine_id);
            if provider == "feishu" {
                builder = builder.provider_config(json!({"mode":"new","domain":"feishu"}));
            }
            if provider == "telegram" {
                builder = builder.provider_config(json!({"mode":"new","bot_username":"alpha_bot"}));
            }
            builder.create().await?;
        }
        Command::Mention {
            workspace,
            agent,
            chat,
            person,
            text,
        } => {
            let team = rig.existing_workspace(&workspace);
            team.mention(&team.existing_agent(&agent), &chat, &person, &text)
                .await?;
        }
        Command::HoldNextRun { workspace, agent } => {
            rig.existing_workspace(&workspace)
                .existing_agent(&agent)
                .view()
                .await?;
            let dir = rig
                .runtime_home()
                .join("scripted-members")
                .join(&workspace)
                .join(&agent);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("hold"), b"hold")?;
        }
        Command::ReleaseRun { workspace, agent } => {
            rig.existing_workspace(&workspace)
                .existing_agent(&agent)
                .view()
                .await?;
            let gate = rig
                .runtime_home()
                .join("scripted-members")
                .join(&workspace)
                .join(&agent)
                .join("hold");
            match std::fs::remove_file(gate) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
        }
        Command::WaitRunning { workspace, agent } => {
            let team = rig.existing_workspace(&workspace);
            let member = team.existing_agent(&agent);
            member.wait_until_running().await?;
            let execution = member
                .executions()
                .await?
                .into_iter()
                .rev()
                .find(|run| run.state == "running")
                .ok_or("running execution")?;
            let details = rig.execution_details(execution.id).await?;
            let machine = details.attempts.last().ok_or("execution attempt")?;
            return Ok(json!({"machine_id": machine.machine_id}));
        }
        Command::Settle => {
            rig.settle().await?;
            let mut outbound: Vec<Value> = rig.outbound().into_iter().map(|message|
                json!({"workspace": message.workspace, "account": message.account,
                    "chat": message.chat, "channel_type": message.channel_type, "content": message.content})
            ).collect();
            outbound.extend(im.outbound());
            return Ok(json!({"outbound":outbound}));
        }
    }
    Ok(json!({}))
}

#[tokio::main]
async fn main() -> Result<()> {
    let bin = std::env::current_exe()?
        .parent()
        .ok_or("binary directory")?
        .to_path_buf();
    let im = web_im::Im::default();
    let rig = SystemRig::start_with_providers(
        RigConfig {
            harness_executable: bin.join("system-harness"),
            machine_process: Some(bin.join("system-machine")),
            ..Default::default()
        },
        im.factories()?,
    )
    .await?;
    let mut machines = web_machines::Machines::new();
    let scenario = async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        println!("{}", json!({
            "baseURL": std::fs::read_to_string(rig.runtime_run().join(URL_FILE))?.trim(),
            "controlPort": listener.local_addr()?.port(),
            "runtimeHome": rig.runtime_home(),
            "machineHome": rig.machine_home(),
        }));
        std::io::stdout().flush()?;
        // The Playwright parent owns stdin. EOF also cleans up after parent failure.
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            use std::io::Read;
            let _ = std::io::stdin().read_to_end(&mut Vec::new());
            let _ = stop.send(());
        });
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                connection = listener.accept() => {
                    let (socket, _) = connection?;
                    let exchange = async {
                        let mut socket = BufReader::new(socket);
                        let mut line = String::new();
                        socket.read_line(&mut line).await?;
                        let result = match serde_json::from_str::<Command>(&line) {
                            Ok(command) => control(&rig, &mut machines, &im, command).await,
                            Err(error) => Err(error.into()),
                        };
                        let reply = match result {
                            Ok(data) => json!({"ok": true, "data": data}),
                            Err(error) => json!({"ok": false, "error": error.to_string()}),
                        };
                        socket.write_all(format!("{reply}\n").as_bytes()).await?;
                        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
                    };
                    // A broken client must not prevent teardown or subsequent scenarios.
                    tokio::select! {
                        _ = &mut stopped => break,
                        result = tokio::time::timeout(Duration::from_secs(30), exchange) => {
                            if let Ok(Err(error)) = result { eprintln!("web rig control: {error}"); }
                        }
                    }
                }
            }
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    // Scripted peers have separate process groups. A held peer cannot read EOF,
    // so stop owned peer groups before killing the Machine and removing its home.
    for home in std::iter::once(rig.machine_home().to_path_buf()).chain(machines.homes.clone()) {
        let journal = home.join("harness-pids.tsv");
        let text = std::fs::read_to_string(journal).unwrap_or_default();
        for pid in text
            .lines()
            .filter_map(|line| line.strip_prefix("spawned\t")?.parse::<u32>().ok())
        {
            let _ = std::process::Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{pid}")])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?;
        }
    }
    let remote_cleanup = machines.stop();
    let cleanup = rig.close().await;
    remote_cleanup?;
    scenario?;
    cleanup
}
