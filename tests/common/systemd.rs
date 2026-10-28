//! A stand-in for systemd's user manager, for the tests that install the
//! autosave timer (`autosave install`, `setup`): [`FakeSystemd`] and the
//! [`Units`] it holds. systemd itself isn't exercised, so these tests run on
//! macOS too, with the systemd scheduler picked by `REXURRECT_SCHEDULER`.

use std::path::{Path, PathBuf};

use super::env::TestEnv;
use super::rexurrect::{RexurrectCmd, RunOutput};
use super::scheduled::{run_job, write_script};

const SCHEDULER_VAR: &str = "REXURRECT_SCHEDULER";
const UNIT_DIR_VAR: &str = "REXURRECT_SYSTEMD_USER_DIR";
pub const NAME: &str = "rexurrect-autosave";

/// A stand-in for the user's systemd manager: a unit directory and a
/// `systemctl --user` that models `daemon-reload`, `enable`, `restart` and
/// `disable --now` on it the way the real one answers (only units loaded by
/// the last `daemon-reload` can be enabled or started). It keeps a copy of
/// each started unit as it was loaded, as systemd does.
pub struct FakeSystemd {
    dir: PathBuf,
}

impl FakeSystemd {
    pub fn new(env: &TestEnv) -> FakeSystemd {
        FakeSystemd::new_named(env, "systemd")
    }

    /// A second stand-in in the same `TestEnv`, in its own scratch
    /// directory `name`, to compare what two installs leave.
    pub fn new_named(env: &TestEnv, name: &str) -> FakeSystemd {
        let dir = env.mkdir(name);
        for sub in ["bin", "loaded", "enabled", "active", "user", "home"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        write_script(&dir.join("bin/systemctl"), &systemctl_script(&dir));
        write_script(&dir.join("bin/launchctl"), NO_LAUNCHCTL);
        FakeSystemd { dir }
    }

    /// `cmd` with the systemd scheduler, its unit directory and `systemctl`
    /// pointed here.
    pub fn aim(&self, cmd: RexurrectCmd) -> RexurrectCmd {
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
    pub fn aim_at_systemctl_only(&self, cmd: RexurrectCmd) -> RexurrectCmd {
        cmd.env(SCHEDULER_VAR, "systemd")
            .env("HOME", self.home())
            .env("REXURRECT_LAUNCH_AGENTS_DIR", self.home())
            .path_prepend(self.dir.join("bin"))
    }

    pub fn unit_dir(&self) -> PathBuf {
        self.dir.join("user")
    }

    /// The `HOME` the runs it aims get.
    pub fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    /// Makes every `enable` from now on fail, the way systemd refuses a unit
    /// it can't load.
    pub fn refuse_to_enable(&self) {
        std::fs::write(self.dir.join("refuse-enable"), "").unwrap();
    }

    /// The `systemctl` calls made so far, one line each.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    pub fn names_in(&self, sub: &str) -> Vec<String> {
        names_in(&self.dir.join(sub))
    }

    /// The service and timer installed, enabled and started, checking that
    /// they're the only units installed, and that the timer running is the
    /// one installed.
    pub fn the_units(&self) -> Units {
        self.the_units_in(&self.unit_dir(), NAME)
    }

    pub fn the_units_in(&self, unit_dir: &Path, name: &str) -> Units {
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

pub fn names_in(dir: &Path) -> Vec<String> {
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
#[derive(Debug, PartialEq)]
pub struct Units {
    pub service: Unit,
    pub timer: Unit,
}

/// A unit file's settings: `(section, key, value)`, in order.
#[derive(Debug, PartialEq)]
pub struct Unit {
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
    pub fn get(&self, section: &str, key: &str) -> &str {
        match self.all(section, key)[..] {
            [value] => value,
            ref values => panic!("[{section}] {key}: {values:?} in {self:?}"),
        }
    }
}

impl Units {
    /// The service's command line, unquoted as systemd reads it.
    pub fn exec_start(&self) -> Vec<String> {
        words(self.service.get("Service", "ExecStart"), true)
    }

    /// The service's environment, unquoted as systemd reads it.
    pub fn environment(&self) -> Vec<(String, String)> {
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

    pub fn env_var(&self, name: &str) -> Option<String> {
        self.environment()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, value)| value)
    }

    /// How often the timer runs the service, in seconds.
    pub fn period(&self) -> u64 {
        seconds(self.timer.get("Timer", "OnUnitActiveSec"))
    }

    /// Runs the service the way systemd's user manager runs it: its command
    /// line, with the manager's own small environment plus the service's
    /// `Environment=` (see [`run_job`]).
    pub fn run_as_systemd(&self, env: &TestEnv) -> RunOutput {
        run_job(
            env,
            "systemd",
            &self.exec_start(),
            &["USER", "LOGNAME", "SHELL"],
            self.environment(),
        )
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
