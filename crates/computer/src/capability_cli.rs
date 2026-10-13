use crate::ComputerError;
use lankide_harness::HarnessConfig;
use lankide_protocol::CliCommand;
use std::path::{Path, PathBuf};

pub(crate) fn discover() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let mut candidates = vec![executable.with_file_name("lankide")];
    if let Some(contents) = executable
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "Contents"))
    {
        candidates.push(contents.join("Resources/bin/lankide"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|directory| directory.join("lankide")));
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
    config: &mut HarnessConfig,
    commands: &[CliCommand],
    prompt: &mut String,
) -> Result<(), ComputerError> {
    let Some(executable) = executable else {
        if commands.is_empty() {
            return Ok(());
        }
        return Err(ComputerError::InvalidAssignment(
            "capability_cli_unavailable".into(),
        ));
    };
    let executable = executable.canonicalize()?;
    if !executable_file(&executable) {
        return Err(ComputerError::InvalidAssignment(
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
    let mut paths = vec![directory.to_path_buf()];
    paths.extend(std::env::split_paths(&inherited).filter(|path| path != directory));
    let path = std::env::join_paths(paths)
        .map_err(|_| ComputerError::InvalidAssignment("invalid_capability_cli_path".into()))?;
    config
        .environment
        .insert("PATH".into(), path.to_string_lossy().into_owned());
    config.environment.insert(
        "LANKIDE_CLI".into(),
        executable.to_string_lossy().into_owned(),
    );
    prompt.push_str(
        "\n\n## 可用命令\n\n- `lankide capabilities`：查看可用操作和参数。\n- `lankide profile`：查看自己的姓名、岗位、签名和头像。\n- `lankide profile update --name NAME --role ROLE --bio BIO --avatar PATH`：更新自己的资料；只传需要修改的字段。\n",
    );
    for command in commands {
        // The CLI remains the single gateway client; Provider commands do not
        // introduce per-Agent executables or direct credential access.
        let capability = format!("cli: {}", command.name).replace('\'', "'\"'\"'");
        prompt.push_str(&format!(
            "\n- {}：{}\n  调用：`lankide invoke '{}' --arguments-json <JSON参数数组>`\n",
            command.name, command.description, capability,
        ));
    }
    Ok(())
}
