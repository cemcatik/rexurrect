//! Autosave on Linux: `rexurrect autosave install` puts a systemd user timer
//! in place (a `.service` and a `.timer` unit) that runs `save` on a timer,
//! and `autosave uninstall` removes it.
//!
//! systemd itself isn't exercised, so these run on macOS too, with the
//! systemd scheduler picked by `REXURRECT_SCHEDULER`. Install and uninstall
//! run with their unit directory pointed at a temp dir and a stand-in
//! `systemctl` first on `PATH` ([`FakeSystemd`]) that keeps track of which
//! units are loaded, enabled and started. The installed service is then run
//! the way systemd would run it ([`Units::run_as_systemd`]): its command
//! line and environment only, so no `REX_SERVER` from the test leaks in.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;

const SCHEDULER_VAR: &str = "REXURRECT_SCHEDULER";
const UNIT_DIR_VAR: &str = "REXURRECT_SYSTEMD_USER_DIR";
const NAME: &str = "rexurrect-autosave";

/// A stand-in for the user's systemd manager: a unit directory and a
/// `systemctl --user` that models `daemon-reload`, `enable`, `restart` and
/// `disable --now` on it the way the real one answers (only units loaded by
/// the last `daemon-reload` can be enabled or started). It keeps a copy of
/// each started unit as it was loaded, as systemd does.
struct FakeSystemd {
    dir: PathBuf,
}

impl FakeSystemd {
    fn new(env: &TestEnv) -> FakeSystemd {
        let dir = env.mkdir("systemd");
        for sub in ["bin", "loaded", "enabled", "active", "user", "home"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        for (name, script) in [
            ("systemctl", systemctl_script(&dir)),
            ("launchctl", NO_LAUNCHCTL.to_string()),
        ] {
            let path = dir.join("bin").join(name);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        FakeSystemd { dir }
    }

    /// `cmd` with the systemd scheduler, its unit directory and `systemctl`
    /// pointed here.
    fn aim(&self, cmd: RexurrectCmd) -> RexurrectCmd {
        self.aim_at_systemctl_only(cmd)
            .env(UNIT_DIR_VAR, self.unit_dir())
            .env("UNIT_DIR", self.unit_dir())
    }

    /// `cmd` with the systemd scheduler and `systemctl` pointed here, but
    /// the unit directory left to the defaults. The stand-in `systemctl`
    /// loads units from `$UNIT_DIR`, so set that to where they should go.
    ///
    /// Safety: should a run ever pick launchd instead, it finds a temp
    /// `HOME` and LaunchAgents directory, and a `launchctl` that refuses
    /// every call, never the user's own agent.
    fn aim_at_systemctl_only(&self, cmd: RexurrectCmd) -> RexurrectCmd {
        cmd.env(SCHEDULER_VAR, "systemd")
            .env("HOME", self.dir.join("home"))
            .env("REXURRECT_LAUNCH_AGENTS_DIR", self.dir.join("home"))
            .path_prepend(self.dir.join("bin"))
    }

    fn unit_dir(&self) -> PathBuf {
        self.dir.join("user")
    }

    /// Makes every `enable` from now on fail, the way systemd refuses a unit
    /// it can't load.
    fn refuse_to_enable(&self) {
        std::fs::write(self.dir.join("refuse-enable"), "").unwrap();
    }

    /// The `systemctl` calls made so far, one line each.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn names_in(&self, sub: &str) -> Vec<String> {
        names_in(&self.dir.join(sub))
    }

    /// The service and timer installed, enabled and started, checking that
    /// they're the only units installed, and that the timer running is the
    /// one installed.
    fn the_units(&self) -> Units {
        self.the_units_in(&self.unit_dir(), NAME)
    }

    fn the_units_in(&self, unit_dir: &Path, name: &str) -> Units {
        let service = format!("{name}.service");
        let timer = format!("{name}.timer");
        assert_eq!(
            names_in(unit_dir),
            [service.as_str(), timer.as_str()],
            "installed units"
        );
        assert_eq!(self.names_in("enabled"), [timer.as_str()], "enabled units");
        assert_eq!(self.names_in("active"), [timer.as_str()], "started units");
        let timer_text = read(&unit_dir.join(&timer));
        assert_eq!(
            read(&self.dir.join("active").join(&timer)),
            timer_text,
            "the timer started is the one installed"
        );
        Units {
            service: Unit::parse(&read(&unit_dir.join(&service))),
            timer: Unit::parse(&timer_text),
        }
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

/// A `launchctl` that refuses every call: these tests never reach launchd.
const NO_LAUNCHCTL: &str = "#!/bin/sh
echo \"stand-in launchctl: the systemd tests picked launchd: $*\" >&2
exit 64
";

fn systemctl_script(dir: &Path) -> String {
    format!(
        r#"#!/bin/sh
# Stand-in systemctl (rexurrect test harness).
state='{dir}'
printf '%s\n' "$*" >> "$state/calls"
if [ "$1" != --user ]; then
  echo "stand-in systemctl: not a --user call: $*" >&2
  exit 64
fi
shift
case "$1" in
  daemon-reload)
    rm -f "$state/loaded/"*
    for unit in "$UNIT_DIR"/*; do
      [ -f "$unit" ] && cp "$unit" "$state/loaded/"
    done
    exit 0 ;;
  enable)
    if [ -f "$state/refuse-enable" ]; then
      echo "Failed to enable unit: Unit $2 is masked." >&2
      exit 1
    fi
    if [ ! -f "$state/loaded/$2" ]; then
      echo "Failed to enable unit: Unit file $2 does not exist." >&2
      exit 1
    fi
    touch "$state/enabled/$2" ;;
  restart)
    if [ ! -f "$state/loaded/$2" ]; then
      echo "Failed to restart $2: Unit $2 not found." >&2
      exit 5
    fi
    cp "$state/loaded/$2" "$state/active/$2" ;;
  disable)
    if [ "$2" != --now ]; then
      echo "stand-in systemctl: unexpected call: $*" >&2
      exit 64
    fi
    if [ ! -f "$state/loaded/$3" ]; then
      echo "Failed to disable unit: Unit file $3 does not exist." >&2
      exit 1
    fi
    rm -f "$state/enabled/$3" "$state/active/$3" ;;
  *)
    echo "stand-in systemctl: unexpected call: --user $*" >&2
    exit 64 ;;
esac
"#,
        dir = dir.display()
    )
}

/// The installed service and timer.
#[derive(Debug)]
struct Units {
    service: Unit,
    timer: Unit,
}

/// A unit file's settings: `(section, key, value)`, in order.
#[derive(Debug)]
struct Unit {
    settings: Vec<(String, String, String)>,
}

impl Unit {
    fn parse(text: &str) -> Unit {
        let mut section = String::new();
        let mut settings = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.to_string();
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("not a setting: {line:?}"));
            settings.push((section.clone(), key.trim().into(), value.trim().into()));
        }
        Unit { settings }
    }

    /// Every value of `key` in `section`.
    fn all(&self, section: &str, key: &str) -> Vec<&str> {
        self.settings
            .iter()
            .filter(|(s, k, _)| s == section && k == key)
            .map(|(_, _, v)| v.as_str())
            .collect()
    }

    /// The one value of `key` in `section`.
    fn get(&self, section: &str, key: &str) -> &str {
        match self.all(section, key)[..] {
            [value] => value,
            ref values => panic!("[{section}] {key}: {values:?} in {self:?}"),
        }
    }
}

impl Units {
    /// The service's command line, unquoted as systemd reads it.
    fn exec_start(&self) -> Vec<String> {
        words(self.service.get("Service", "ExecStart"), true)
    }

    /// The service's environment, unquoted as systemd reads it.
    fn environment(&self) -> Vec<(String, String)> {
        self.service
            .all("Service", "Environment")
            .into_iter()
            .flat_map(|line| words(line, false))
            .map(|assignment| {
                let (name, value) = assignment.split_once('=').unwrap();
                (name.to_string(), value.to_string())
            })
            .collect()
    }

    fn env_var(&self, name: &str) -> Option<String> {
        self.environment()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, value)| value)
    }

    /// How often the timer runs the service, in seconds.
    fn period(&self) -> u64 {
        seconds(self.timer.get("Timer", "OnUnitActiveSec"))
    }

    /// Runs the service the way systemd's user manager runs it: its command
    /// line, in `/`, with the manager's own small environment (no
    /// `REX_SERVER`, no state directory override, a bare `PATH`) plus the
    /// service's `Environment=`.
    ///
    /// Safety: `HOME` is a temp dir, so if the service didn't name a server
    /// or a state directory, the job would fall back to defaults under that
    /// temp dir, never the real ones; so does `XDG_RUNTIME_DIR`, which the
    /// user manager sets.
    fn run_as_systemd(&self, env: &TestEnv) -> RunOutput {
        let args = self.exec_start();
        let harness = env.child_env();
        let out = Command::new(&args[0])
            .args(&args[1..])
            .env_clear()
            .env("HOME", env.mkdir("systemd-home"))
            .env("PATH", SYSTEM_PATH)
            .env(
                "XDG_RUNTIME_DIR",
                &harness[std::ffi::OsStr::new("XDG_RUNTIME_DIR")],
            )
            .envs(["USER", "LOGNAME", "SHELL"].iter().filter_map(|var| {
                harness
                    .get(std::ffi::OsStr::new(var))
                    .map(|value| (var, value))
            }))
            .envs(self.environment())
            .current_dir("/")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        RunOutput {
            command: format!("the service's job {args:?}"),
            status: out.status,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }
}

/// A time span as systemd writes one, in this harness only ever seconds:
/// `900s` or `900`.
fn seconds(span: &str) -> u64 {
    span.strip_suffix('s')
        .unwrap_or(span)
        .parse()
        .unwrap_or_else(|_| panic!("not a time span in seconds: {span:?}"))
}

/// The words of a unit setting, as systemd splits and unquotes them: words
/// split on spaces, double quotes around a word, C escapes inside quotes,
/// `%%` for `%` and, in a command line (`exec`), `$$` for `$`. Anything
/// else unescaped that systemd would expand (`%h`, `$HOME`) fails the test.
fn words(line: &str, exec: bool) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        let Some(&first) = chars.peek() else {
            return words;
        };
        let quoted = first == '"';
        if quoted {
            chars.next();
        }
        let mut word = String::new();
        loop {
            let Some(c) = chars.next() else {
                assert!(!quoted, "unterminated quote in {line:?}");
                break;
            };
            match c {
                '"' if quoted => break,
                ' ' if !quoted => break,
                '\\' if quoted => match chars.next() {
                    Some('\\') => word.push('\\'),
                    Some('"') => word.push('"'),
                    Some('n') => word.push('\n'),
                    Some('t') => word.push('\t'),
                    other => panic!("unexpected escape \\{other:?} in {line:?}"),
                },
                '%' => {
                    assert_eq!(chars.next(), Some('%'), "a specifier in {line:?}");
                    word.push('%');
                }
                '$' if exec => {
                    assert_eq!(chars.next(), Some('$'), "a variable in {line:?}");
                    word.push('$');
                }
                c => word.push(c),
            }
        }
        words.push(word);
    }
}

/// An endpoint for install tests that never run the service.
const NO_SERVER: &str = "unix:///tmp/rxt-none/s.sock";

#[test]
fn install_starts_a_timer_that_saves_every_15_minutes_by_default() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    let out = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let units = systemd.the_units();
    assert_eq!(units.exec_start(), [common::rexurrect::bin(), "save"]);
    assert_eq!(units.period(), 15 * 60);
    assert_eq!(units.timer.get("Install", "WantedBy"), "timers.target");
    // Install says what it installed, and where.
    assert!(out.stdout.contains("systemd user timer"), "{out}");
    assert!(out.stdout.contains(&format!("{NAME}.timer")), "{out}");
    assert!(
        out.stdout.contains(systemd.unit_dir().to_str().unwrap()),
        "{out}"
    );
}

#[test]
fn install_reloads_units_before_enabling_and_starting_the_timer() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    assert_eq!(
        systemd.calls(),
        [
            "--user daemon-reload".to_string(),
            format!("--user enable {NAME}.timer"),
            format!("--user restart {NAME}.timer"),
        ]
    );
}

#[test]
fn the_interval_can_be_changed_at_install() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install", "--interval", "5"])
        .run()
        .assert_success();

    assert_eq!(systemd.the_units().period(), 5 * 60);
}

#[test]
fn installing_twice_leaves_exactly_one_timer_running_with_the_latest_settings() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let install = |args: &[&str]| {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["autosave", "install"])
            .args(args)
            .run()
            .assert_success()
    };

    install(&["--interval", "5"]);
    install(&[]);

    // the_units checks there's one of each installed and that the timer
    // started is the one installed: the second install restarted it.
    assert_eq!(systemd.the_units().period(), 15 * 60);
}

#[test]
fn an_autosave_saves_the_server_and_state_install_ran_with_though_systemd_gives_neither() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let server = env.start_server();
    let fixture = server.create_session("zz-autosaved", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    systemd
        .aim(env.rexurrect(&server))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    systemd.the_units().run_as_systemd(&env).assert_success();

    // The snapshot went to the state directory install ran with.
    server.kill_session(&fixture.id);
    env.restore(&server);
    assert_eq!(server.session_ids_labelled("zz-autosaved").len(), 1);
}

/// On Linux, with no `REX_SERVER` at install (outside Rex), autosave saves
/// the server `rex` itself defaults to: `$XDG_RUNTIME_DIR/rex/server.sock`,
/// where the login server's per-account server listens.
#[cfg(target_os = "linux")]
#[test]
fn installed_outside_rex_autosave_saves_the_server_at_rexs_default_endpoint() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let runtime = env.mkdir("run user");

    let install = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .env_remove("REX_SERVER")
        .env("XDG_RUNTIME_DIR", &runtime)
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let endpoint = format!(
        "unix://{}/rex/server.sock",
        runtime.to_str().unwrap().replace(' ', "%20")
    );
    assert_eq!(
        systemd.the_units().env_var("REX_SERVER"),
        Some(endpoint.clone())
    );
    assert!(install.stdout.contains(&endpoint), "{install}");
}

#[test]
fn a_successful_autosave_is_silent() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let server = env.start_server();
    systemd
        .aim(env.rexurrect(&server))
        .args(["autosave", "install"])
        .run()
        .assert_success();
    let units = systemd.the_units();

    units.run_as_systemd(&env).assert_success();

    // Nothing goes to the journal every interval; failures go to the log.
    assert_eq!(units.service.get("Service", "StandardOutput"), "null");
    assert_eq!(units.service.get("Service", "StandardError"), "null");
    assert_eq!(env.log_entries(), Vec::<String>::new());
}

#[test]
fn values_systemd_would_expand_reach_the_job_as_they_were() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let state = env.mkdir("state 100%h $HOME \"q\" \\b");
    let programs = "claude %n $USER \"x\" \\y";

    systemd
        .aim(env.rexurrect_at("unix:///tmp/rxt%20none/s.sock"))
        .env(STATE_DIR_VAR, &state)
        .env("REXURRECT_NO_SCROLLBACK", programs)
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let units = systemd.the_units();
    assert_eq!(
        units.env_var("REX_SERVER").as_deref(),
        Some("unix:///tmp/rxt%20none/s.sock")
    );
    assert_eq!(
        units.env_var(STATE_DIR_VAR).as_deref(),
        Some(state.to_str().unwrap())
    );
    assert_eq!(
        units.env_var("REXURRECT_NO_SCROLLBACK").as_deref(),
        Some(programs)
    );
}

/// Asserts `out` tells the user how to remove the zsh recorder and the
/// auto-restore line in Rex's init.lua.
fn assert_explains_backing_out(out: &RunOutput) {
    for what in [
        "rexurrect.plugin.zsh",
        ".zshrc",
        "init.lua",
        "https://github.com/cemcatik/rexurrect#auto-restore",
    ] {
        assert!(out.stdout.contains(what), "no mention of {what}: {out}");
    }
}

#[test]
fn uninstall_stops_and_removes_the_timer_and_says_how_to_back_out_of_the_rest() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let out = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "uninstall"])
        .run()
        .assert_success();

    assert!(
        systemd.names_in("enabled").is_empty(),
        "the timer was disabled"
    );
    assert!(
        systemd.names_in("active").is_empty(),
        "the timer was stopped"
    );
    assert!(
        names_in(&systemd.unit_dir()).is_empty(),
        "its units were removed"
    );
    assert!(systemd.names_in("loaded").is_empty(), "and unloaded");
    assert!(out.stdout.contains("autosave uninstalled"), "{out}");
    assert_explains_backing_out(&out);
}

#[test]
fn uninstall_without_a_timer_changes_nothing_and_still_says_how_to_back_out() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let uninstall = || {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["autosave", "uninstall"])
            .run()
            .assert_success()
    };

    let out = uninstall();

    assert!(out.stdout.contains("wasn't installed"), "{out}");
    assert_explains_backing_out(&out);
    assert_eq!(systemd.calls(), Vec::<String>::new());
}

#[test]
fn a_second_uninstall_says_there_was_nothing_to_remove() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let autosave = |action: &str| {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["autosave", action])
            .run()
            .assert_success()
    };

    autosave("install");
    autosave("uninstall");
    let out = autosave("uninstall");

    assert!(out.stdout.contains("wasn't installed"), "{out}");
}

#[test]
fn a_test_label_keeps_a_scratch_timer_apart_from_the_real_one() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let scratch = "rexurrect-autosave-test-1234";
    let autosave = |action: &str| {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .env("REXURRECT_AUTOSAVE_LABEL", scratch)
            .args(["autosave", action])
            .run()
            .assert_success()
    };

    autosave("install");
    systemd.the_units_in(&systemd.unit_dir(), scratch);

    autosave("uninstall");
    assert!(names_in(&systemd.unit_dir()).is_empty());
    assert!(systemd.names_in("enabled").is_empty());
}

#[test]
fn a_label_that_is_not_a_unit_name_installs_nothing() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    for bad in ["a/b", "a b", "a@b", "x.timer\nExecStart=/bin/sh"] {
        let out = systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .env("REXURRECT_AUTOSAVE_LABEL", bad)
            .args(["autosave", "install"])
            .run()
            .assert_failure();
        assert!(out.stderr.contains("REXURRECT_AUTOSAVE_LABEL"), "{out}");
    }
    assert!(names_in(&systemd.unit_dir()).is_empty());
}

#[test]
fn units_go_in_xdg_config_home_else_dot_config() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let home = env.mkdir("home");
    let config = env.mkdir("config");

    // The fake systemctl loads units from where install put them.
    systemd
        .aim_at_systemctl_only(env.rexurrect_at(NO_SERVER))
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config)
        .env("UNIT_DIR", config.join("systemd/user"))
        .args(["autosave", "install"])
        .run()
        .assert_success();
    systemd.the_units_in(&config.join("systemd/user"), NAME);

    systemd
        .aim_at_systemctl_only(env.rexurrect_at(NO_SERVER))
        .env("HOME", &home)
        .env("UNIT_DIR", config.join("systemd/user"))
        .args(["autosave", "uninstall"])
        .run()
        .assert_success();

    systemd
        .aim_at_systemctl_only(env.rexurrect_at(NO_SERVER))
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env("UNIT_DIR", home.join(".config/systemd/user"))
        .args(["autosave", "install"])
        .run()
        .assert_success();
    systemd.the_units_in(&home.join(".config/systemd/user"), NAME);
}

/// An enabled timer starts at every login, so units systemd refused to
/// enable mustn't stay installed.
#[test]
fn a_timer_systemd_fails_to_enable_is_not_left_installed() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    systemd.refuse_to_enable();

    let out = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_failure();

    assert!(out.stderr.contains("masked"), "{out}");
    assert!(
        names_in(&systemd.unit_dir()).is_empty(),
        "its units were removed"
    );
    assert!(systemd.names_in("loaded").is_empty(), "and unloaded");
}

/// Rex moves its CLI on updates, so the service doesn't pin one: with no
/// `rex` on its `PATH`, save runs the one a running Rex server runs
/// (ADR-0003).
#[test]
fn the_service_gets_only_the_system_path() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    assert_eq!(
        systemd.the_units().env_var("PATH").as_deref(),
        Some(SYSTEM_PATH)
    );
}

#[test]
fn autosaves_keep_as_many_snapshots_as_install_was_told_to() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let server = env.start_server();
    let fixture = server.create_session("zz-kept", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    systemd
        .aim(env.rexurrect(&server))
        .env("REXURRECT_KEEP", "2")
        .args(["autosave", "install"])
        .run()
        .assert_success();
    let units = systemd.the_units();

    for _ in 0..3 {
        units.run_as_systemd(&env).assert_success();
    }

    let list = env.rexurrect(&server).arg("list").run().assert_success();
    assert_eq!(list.stdout.lines().count(), 2, "{list}");
}
