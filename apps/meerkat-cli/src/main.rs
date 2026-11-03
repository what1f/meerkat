#![forbid(unsafe_code)]

use meerkat_cli::ExecOutcome;

#[tokio::main]
async fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().map(String::as_str) == Some("exec") {
        exec(&arguments[1..]).await;
    }
    match meerkat_cli::execute_from_environment(arguments).await {
        Ok(output) => println!(
            "{}",
            serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_owned())
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

async fn exec(arguments: &[String]) -> ! {
    match meerkat_cli::prepare_exec_from_environment(arguments).await {
        Ok(ExecOutcome::Run {
            program,
            arguments,
            environment,
        }) => {
            let mut command = std::process::Command::new(&program);
            command.args(arguments).envs(environment);
            #[cfg(unix)]
            let error = std::os::unix::process::CommandExt::exec(&mut command);
            #[cfg(not(unix))]
            let error = match command.status() {
                Ok(status) => std::process::exit(status.code().unwrap_or(1)),
                Err(error) => error,
            };
            eprintln!("{program}: {error}");
            std::process::exit(127);
        }
        Ok(ExecOutcome::Refused(message)) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}
