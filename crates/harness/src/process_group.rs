/// Stop all processes in an isolated native Harness process group, including
/// descendants that remain after the CLI leader exits.
pub fn kill_process_group(id: u32) {
    #[cfg(unix)]
    if let Some(group) = rustix::process::Pid::from_raw(id as i32) {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    #[cfg(not(unix))]
    let _ = id;
}
