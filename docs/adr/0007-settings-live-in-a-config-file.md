# Settings live in a config file

rexurrect's settings move from environment variables to one TOML file, `$XDG_CONFIG_HOME/rexurrect/config.toml` (or `~/.config/rexurrect/config.toml`), which `REXURRECT_CONFIG` can replace. `REXURRECT_KEEP`, `REXURRECT_NO_SCROLLBACK`, `REXURRECT_READY_TIMEOUT` and `REXURRECT_APP_SESSION_WATCH` become `keep`, `no_scrollback`, `ready_timeout` and `app_session_watch`, and the resume work's settings (`auto_resume`, program definitions) are only ever in the file. It's a clean break: the old variables are no longer read, and a run that finds one set warns, naming its replacement. `autosave install` and `auto-restore --print-hook` pin the file's path, never its values. Issue #53 carries this out.

Autosave runs under launchd or systemd and auto-restore inside the Rex server, and neither sees the user's shell environment. So far `autosave install` copied settings into the autosave job as they were when it ran, and auto-restore saw only what its `init.lua` line set: a setting changed later applied to manual runs while autosave and auto-restore silently kept the old value. Restore now has to read a list of programs to auto-resume and tables of program definitions with quoted templates, which environment variables carry badly, and restore is exactly the part that runs in Rex's environment.

## Considered Options

- **Environment variables for everything, tmux-resurrect style** (a space-separated list, a `->` syntax for rewrites): consistent with what's there, but templates with quotes inside shell variables are painful, and every change needs a reinstall that's easy to forget.
- **The file, with environment variables overriding it**: nothing breaks, but the variables `autosave install` already pinned into launchd plists would keep silently beating the file, the exact confusion the file is meant to end.
- **Executable plugins for program definitions**: the most flexible, but rexurrect would run arbitrary code during every save and restore for what no program yet needs; a definition can grow a field for computed state later.
- **The file, as a clean break, with a warning naming each replacement** (chosen): as ADR-0002's rename did, few installs exist yet, and a specific warning makes the break cheap.

## Consequences

- Some settings stay environment variables, for reasons the file can't remove: `REXURRECT_STATE_DIR`, because the zsh recorder resolves the state directory itself at every command and can't read TOML, and the two must agree; `REXURRECT_IGNORE`, which only the zsh recorder reads; `REXURRECT_CONFIG`; and the test-only overrides.
- Finding the file has the same problem finding the state directory has, so its path is pinned the same way. Edits to the file take effect without reinstalling.
- Existing installs keep working with defaults and warn until the variables are moved into the file, and autosave is reinstalled so its job stops carrying them.
- Tests set settings by writing a config file through `REXURRECT_CONFIG` instead of setting variables.
