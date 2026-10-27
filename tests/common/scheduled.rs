//! What the autosave tests share across schedulers (launchd, systemd): an
//! endpoint with no server, running an installed job the way its scheduler
//! would, and the backing-out advice `autosave uninstall` prints.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use super::env::{SYSTEM_PATH, TestEnv};
use super::rexurrect::RunOutput;

/// An endpoint for install tests that never run the job.
pub const NO_SERVER: &str = "unix:///tmp/rxt-none/s.sock";

/// Writes an executable script, e.g. a stand-in `launchctl` or `systemctl`.
pub fn write_script(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Runs an installed job the way a scheduler runs it: `command`, in `/`,
/// with the scheduler's own small environment (no `REX_SERVER`, no state
/// directory override, a bare `PATH`, `inherited` from the harness) plus the
/// job's `job_env`.
///
/// Safety: `HOME` is a temp dir, so if the job didn't name a server or a
/// state directory, it would fall back to defaults under that temp dir,
/// never the real ones. `XDG_RUNTIME_DIR` points at the harness's empty temp
/// dir for the same reason: `rex` falls back to
/// `$XDG_RUNTIME_DIR/rex/server.sock`.
pub fn run_job<K, V>(
    env: &TestEnv,
    scheduler: &str,
    command: &[impl AsRef<OsStr>],
    inherited: &[&str],
    job_env: impl IntoIterator<Item = (K, V)>,
) -> RunOutput
where
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let harness = env.child_env();
    let inherited = inherited
        .iter()
        .filter_map(|var| harness.get(OsStr::new(var)).map(|value| (var, value)));
    let out = Command::new(&command[0])
        .args(&command[1..])
        .env_clear()
        .env("HOME", env.mkdir(&format!("{scheduler}-home")))
        .env("PATH", SYSTEM_PATH)
        .env("XDG_RUNTIME_DIR", &harness[OsStr::new("XDG_RUNTIME_DIR")])
        .envs(inherited)
        .envs(job_env)
        .current_dir("/")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let shown: Vec<_> = command.iter().map(|arg| arg.as_ref()).collect();
    RunOutput {
        command: format!("the {scheduler} job {shown:?}"),
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Asserts `out` tells the user how to remove the zsh recorder and the
/// auto-restore line in Rex's init.lua.
pub fn assert_explains_backing_out(out: &RunOutput) {
    for what in [
        "rexurrect.plugin.zsh",
        ".zshrc",
        "init.lua",
        "https://github.com/cemcatik/rexurrect#auto-restore",
    ] {
        assert!(out.stdout.contains(what), "no mention of {what}: {out}");
    }
}
