# Resume follows the foreground program, and programs report values, not commands

A block resumes its program (an agent conversation, a Neovim session) when the program in its foreground at the save matches a program definition. Programs that need state to resume report it with `rexurrect report <program> [value]`: a value (an ID, a path), never a command. The definition, built in or the user's, turns that value into the resume command. What `report` writes is private to rexurrect; the recorder-file contract stays as it was, written only by shell recorders. Issue #54 carries this out; it supersedes #35 and #50, whose reporter showed that a Claude Code hook or a Neovim plugin could write a whole resume command into a block's recorder file with no rexurrect change.

## Considered Options

- **Programs write their resume command into the block's recorder file** (#35's route A, #50's plugin): works today. But it overwrites the shell recorder's last command, its record outlives the program until the shell records something new, so a block where the agent had been quit an hour earlier still resumes it, and once restore can run commands (ADR-0006), whatever a third-party hook wrote would be run.
- **rexurrect reads programs' own files with no hook** (#35's route B, Claude Code's `~/.claude/sessions/<pid>.json`): nothing to install, but the file is undocumented, and a block's recorder file beats everything else rexurrect knows about it, so for anyone running the zsh recorder (who has `claude` recorded the moment they typed it) this would never apply without changing that precedence.
- **The command line** (#35's route C): only covers conversations started with `--resume`, and misses any switch after.
- **The block's last command decides what resumes**: closer to how restore works today, but it resumes programs the user had already quit, which matters once they can be run.
- **A documented file format hooks write themselves**, like recorder files: no dependency on finding rexurrect from inside the program, but every hook reimplements the state directory, the block check and the atomic write (#35's comment needed 30 lines of bash and `jq`), and the format is frozen.
- **A full command reported by third parties, validated** (herdr's `resume_argv`): herdr needs it because its table of commands is compiled in. Here users write definitions in the config file, so a third party only ever needs to report a value.
- **The foreground program at the save, values through `rexurrect report`** (chosen): what tmux-resurrect (which restores what's running) and herdr (whose hooks report a conversation's ID or path over its socket, while herdr owns the commands) both do.

## Consequences

- A program the user quits after the last save, before the server stops (up to an autosave interval later), doesn't resume when the block's shell has a recorder (#60): the recorder rewrites the block's record each time the prompt comes back, and restore reads a record written since the save as the program having exited. Without a recorder, or for a program quit inside a nested shell, it still resumes.
- A hook needs rexurrect on the program's `PATH`, or names it by full path, which `rexurrect report <program> --print-hook` prints.
- rexurrect owns the validation and the format of resume state, so it can change both, and it's the only place that decides which command a block gets.
- The recorder-file contract stays shell-only and public, for #40's other shells. "Recorder" goes back to meaning the shell's hook only.
- A program that keeps its own state (Neovim) needs no hook: its definition looks for its state at restore (`requires_file`).
