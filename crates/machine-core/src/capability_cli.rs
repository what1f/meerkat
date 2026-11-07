use crate::MachineError;
use meerkat_harness::HarnessConfig;
use meerkat_protocol::CliCommand;
use std::path::{Path, PathBuf};

pub(crate) fn discover() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let mut candidates = vec![executable.with_file_name("meerkat")];
    if let Some(contents) = executable
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "Contents"))
    {
        candidates.push(contents.join("Resources/bin/meerkat"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|directory| directory.join("meerkat")));
    }
    candidates.into_iter().find(|path| executable_file(path))
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn inject(
    executable: Option<&Path>,
    shim_directory: &Path,
    agent_home: &Path,
    config: &mut HarnessConfig,
    commands: &[CliCommand],
    prompt: &mut String,
) -> Result<(), MachineError> {
    // Set before any PATH handling: a call that reaches the real program by
    // another route still finds only the Agent's own directories.
    for command in commands.iter().filter(|command| !command.wraps.is_empty()) {
        for variable in &command.private_directories {
            let valid = !variable.is_empty()
                && variable
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
                && variable != "PATH"
                && !variable.starts_with("MEERKAT_");
            if !valid {
                return Err(MachineError::InvalidAssignment(
                    "invalid_private_directory".into(),
                ));
            }
            let directory = agent_home
                .join(".meerkat")
                .join(&command.wraps)
                .join(variable.to_lowercase());
            std::fs::create_dir_all(&directory)?;
            config
                .environment
                .insert(variable.clone(), directory.to_string_lossy().into_owned());
        }
    }
    let Some(executable) = executable else {
        // Wrapped commands are optional; only ordinary commands need the CLI.
        if commands.iter().all(|command| !command.wraps.is_empty()) {
            return Ok(());
        }
        return Err(MachineError::InvalidAssignment(
            "capability_cli_unavailable".into(),
        ));
    };
    let executable = executable.canonicalize()?;
    if !executable_file(&executable) {
        return Err(MachineError::InvalidAssignment(
            "capability_cli_not_executable".into(),
        ));
    }
    let directory = executable.parent().expect("canonical executable parent");
    let inherited = config
        .environment
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let inherited = std::env::split_paths(&inherited)
        .filter(|path| path != directory && path != shim_directory)
        .collect::<Vec<_>>();
    // A wrapped command is offered only when the user installed its program.
    let mut wrapped = Vec::new();
    for command in commands.iter().filter(|command| !command.wraps.is_empty()) {
        if let Some(target) = find_program(&command.wraps, &inherited) {
            write_shim(shim_directory, &executable, command, &target)?;
            wrapped.push(command);
        }
    }
    let mut paths = Vec::new();
    if !wrapped.is_empty() {
        paths.push(shim_directory.to_path_buf());
    }
    paths.push(directory.to_path_buf());
    let invalid = || MachineError::InvalidAssignment("invalid_capability_cli_path".into());
    // Harnesses whose shell setup rewrites PATH put these entries back in front.
    let own = std::env::join_paths(&paths).map_err(|_| invalid())?;
    paths.extend(inherited);
    let path = std::env::join_paths(paths).map_err(|_| invalid())?;
    config
        .environment
        .insert("PATH".into(), path.to_string_lossy().into_owned());
    config
        .environment
        .insert("MEERKAT_PATH".into(), own.to_string_lossy().into_owned());
    config.environment.insert(
        "MEERKAT_CLI".into(),
        executable.to_string_lossy().into_owned(),
    );
    prompt.push_str(
        "\n\n## 可用命令\n\n- `meerkat capabilities`：查看可用操作和参数。\n- `meerkat profile`：查看自己的姓名、岗位、签名和头像。\n- `meerkat profile update --name NAME --role ROLE --bio BIO --avatar PATH`：更新自己的资料；只传需要修改的字段。\n",
    );
    for command in commands.iter().filter(|command| command.wraps.is_empty()) {
        // The CLI remains the single gateway client; Provider commands do not
        // introduce per-Agent executables or direct credential access.
        let capability = format!("cli: {}", command.name).replace('\'', "'\"'\"'");
        prompt.push_str(&format!(
            "\n- {}：{}\n  调用：`meerkat invoke '{}' --arguments-json <JSON参数数组>`\n",
            command.name, command.description, capability,
        ));
    }
    for command in wrapped {
        prompt.push_str(&format!(
            "\n- `{}`：{}\n",
            command.wraps, command.description
        ));
    }
    Ok(())
}

fn find_program(name: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    paths
        .iter()
        .map(|directory| directory.join(name))
        .find(|path| executable_file(path))
}

/// The shim only routes the call through `meerkat exec`; credentials are
/// fetched per call and never written here.
fn write_shim(
    directory: &Path,
    executable: &Path,
    command: &CliCommand,
    target: &Path,
) -> Result<(), MachineError> {
    let content = format!(
        "#!/bin/sh\nexec {} exec {} -- {} \"$@\"\n",
        shell_quote(&executable.to_string_lossy()),
        shell_quote(&format!("exec: {}", command.name)),
        shell_quote(&target.to_string_lossy()),
    );
    let path = directory.join(&command.wraps);
    if std::fs::read_to_string(&path).is_ok_and(|current| current == content) {
        return Ok(());
    }
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    std::io::Write::write_all(&mut file, content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o755))?;
    }
    file.persist(&path).map_err(|error| error.error)?;
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
