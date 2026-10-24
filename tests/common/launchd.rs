//! A stand-in for launchd, for the tests that install the autosave agent
//! (`autosave install`, `setup`): [`FakeLaunchd`] and the [`Agent`] plists it
//! holds.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::env::{SYSTEM_PATH, TestEnv};
use super::rexurrect::{RexurrectCmd, RunOutput};

pub const AGENTS_DIR_VAR: &str = "REXURRECT_LAUNCH_AGENTS_DIR";
pub const LABEL: &str = "io.github.cemcatik.rexurrect.autosave";

/// A stand-in for the user's launchd domain: a LaunchAgents directory and a
/// `launchctl` that models `print`, `bootstrap` and `bootout` on it the way
/// the real one answers (bootstrapping a loaded agent fails, booting out an
/// agent that isn't loaded fails). It keeps a copy of each loaded agent's
/// plist as it was when bootstrapped, as launchd does.
pub struct FakeLaunchd {
    dir: PathBuf,
}

impl FakeLaunchd {
    pub fn new(env: &TestEnv) -> FakeLaunchd {
        FakeLaunchd::new_named(env, "launchd")
    }

    /// A second stand-in in the same `TestEnv`, in its own scratch
    /// directory `name`, to compare what two installs leave.
    pub fn new_named(env: &TestEnv, name: &str) -> FakeLaunchd {
        let dir = env.mkdir(name);
        for sub in ["bin", "loaded", "LaunchAgents"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let script = dir.join("bin/launchctl");
        std::fs::write(&script, launchctl_script(&dir)).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        FakeLaunchd { dir }
    }

    /// `cmd` with its LaunchAgents directory and `launchctl` pointed here.
    pub fn aim(&self, cmd: RexurrectCmd) -> RexurrectCmd {
        cmd.env(AGENTS_DIR_VAR, self.agents_dir())
            .path_prepend(self.dir.join("bin"))
    }

    pub fn agents_dir(&self) -> PathBuf {
        self.dir.join("LaunchAgents")
    }

    /// Makes every `bootstrap` from now on fail, the way launchd refuses a
    /// plist it can't load.
    pub fn refuse_to_load(&self) {
        std::fs::write(self.dir.join("refuse-bootstrap"), "").unwrap();
    }

    /// The plists in the LaunchAgents directory.
    pub fn installed(&self) -> Vec<Agent> {
        plists_in(&self.agents_dir())
    }

    /// The agents loaded into the fake domain, as they were bootstrapped.
    pub fn loaded(&self) -> Vec<Agent> {
        plists_in(&self.dir.join("loaded"))
    }

    /// The one agent installed and loaded, checking the two agree.
    pub fn the_agent(&self) -> Agent {
        let installed = self.installed();
        let loaded = self.loaded();
        assert_eq!(installed.len(), 1, "installed agents: {installed:?}");
        assert_eq!(loaded.len(), 1, "loaded agents: {loaded:?}");
        assert_eq!(
            installed[0].plist, loaded[0].plist,
            "the loaded agent is the installed one"
        );
        loaded.into_iter().next().unwrap()
    }
}

fn launchctl_script(dir: &Path) -> String {
    format!(
        r#"#!/bin/sh
# Stand-in launchctl (rexurrect test harness).
state='{dir}'
printf '%s\n' "$*" >> "$state/calls"
case "$1" in
  print)
    label=${{2##*/}}
    if [ -f "$state/loaded/$label.plist" ]; then echo "$2 = {{ ... }}"; exit 0; fi
    echo "Could not find service \"$label\" in domain for user gui" >&2
    exit 113 ;;
  bootstrap)
    if [ -f "$state/refuse-bootstrap" ]; then
      echo "Bootstrap failed: 5: Input/output error" >&2
      exit 5
    fi
    label=$(/usr/bin/plutil -extract Label raw -o - "$3") || exit 5
    if [ -f "$state/loaded/$label.plist" ]; then
      echo "Bootstrap failed: 5: Input/output error" >&2
      exit 5
    fi
    cp "$3" "$state/loaded/$label.plist" ;;
  bootout)
    label=${{2##*/}}
    if [ ! -f "$state/loaded/$label.plist" ]; then
      echo "Boot-out failed: 3: No such process" >&2
      exit 3
    fi
    rm "$state/loaded/$label.plist" ;;
  *)
    echo "stand-in launchctl: unexpected call: $*" >&2
    exit 64 ;;
esac
"#,
        dir = dir.display()
    )
}

/// A launchd agent plist, parsed by `plutil`.
#[derive(Debug)]
pub struct Agent {
    pub plist: Value,
}

fn plists_in(dir: &Path) -> Vec<Agent> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    paths.iter().map(|p| Agent::read(p)).collect()
}

impl Agent {
    pub fn read(path: &Path) -> Agent {
        let out = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{} isn't a valid plist: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        Agent {
            plist: serde_json::from_slice(&out.stdout).unwrap(),
        }
    }

    pub fn label(&self) -> &str {
        self.plist["Label"].as_str().unwrap()
    }

    pub fn program_arguments(&self) -> Vec<&str> {
        self.plist["ProgramArguments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect()
    }

    pub fn start_interval(&self) -> u64 {
        self.plist["StartInterval"].as_u64().unwrap()
    }

    /// Where launchd sends the job's stdout and stderr.
    pub fn output_paths(&self) -> [Option<&str>; 2] {
        ["StandardOutPath", "StandardErrorPath"].map(|key| self.plist[key].as_str())
    }

    pub fn environment(&self) -> Vec<(&str, &str)> {
        self.plist["EnvironmentVariables"]
            .as_object()
            .map(|vars| {
                vars.iter()
                    .map(|(k, v)| (k.as_str(), v.as_str().unwrap()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Runs the agent's job the way launchd runs it: its program and
    /// arguments, in `/`, with launchd's own small environment (no
    /// `REX_SERVER`, no state directory override, a bare `PATH`) plus the
    /// agent's `EnvironmentVariables`.
    ///
    /// Safety: `HOME` is a temp dir, so if the agent didn't name a server or
    /// a state directory, the job would fall back to defaults under that
    /// temp dir, never the real ones. `XDG_RUNTIME_DIR` (which launchd
    /// doesn't set) points at the harness's empty temp dir for the same
    /// reason: `rex` falls back to `$XDG_RUNTIME_DIR/rex/server.sock`.
    pub fn run_as_launchd(&self, env: &TestEnv) -> RunOutput {
        let args = self.program_arguments();
        let harness = env.child_env();
        let out = Command::new(args[0])
            .args(&args[1..])
            .env_clear()
            .env("HOME", env.mkdir("launchd-home"))
            .env("PATH", SYSTEM_PATH)
            .env(
                "XDG_RUNTIME_DIR",
                &harness[std::ffi::OsStr::new("XDG_RUNTIME_DIR")],
            )
            .envs(
                ["USER", "LOGNAME", "SHELL", "TMPDIR"]
                    .iter()
                    .filter_map(|var| {
                        harness
                            .get(std::ffi::OsStr::new(var))
                            .map(|value| (var, value))
                    }),
            )
            .envs(self.environment())
            .current_dir("/")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        RunOutput {
            command: format!("the agent's job {args:?}"),
            status: out.status,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }
}
