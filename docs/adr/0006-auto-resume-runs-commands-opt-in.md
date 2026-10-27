# Auto-resume runs commands, opt-in

Restore has never run anything: a block's last command is typed at its prompt, and the user presses Enter. Programs the user lists in `auto_resume` in the config file are the one exception: restore types their resume command followed by Enter, so they come back running in the block's shell. The list is empty by default. A listed program that can't resume (no resume state, no `Session.vim`) comes back running the command line it had at the save, never the block's last command. Programs not listed still get their resume command typed and not run, so everyone gets the right command back without anything running. Issue #59 carries this out.

## Considered Options

- **Never run anything** (the rule so far): safest, but every resumed block needs an Enter after each restart, and agents and editors are what users most want back running. tmux-resurrect restores a conservative list of programs running and herdr resumes agent conversations by itself.
- **Run every program with a definition**: no setup, but an upgrade would start running programs on machines where restore never did, and tmux-resurrect's own docs warn about restoring programs that weren't meant to come back (`sudo mkfs.vfat` reformatting whatever disk is now attached).
- **A default list** (tmux-resurrect's `vi vim view nvim emacs man less more tail top htop irssi weechat mutt`): convenient, but still changes what an upgrade does.
- **A list that gates resuming entirely**, unlisted programs getting their plain last command: simpler to explain, but withholds the right command (`claude --resume <id>`) from everyone who hasn't opted into running it.
- **Rerun the last command when a listed program can't resume**: the last command may only have started the program (`make edit`, `git commit` opening Neovim), and rerunning those is wrong or harmful. The foreground program's own command line is always the program itself.
- **Command blocks** (Rex runs the program instead of a shell): no shell typing involved, but when the program exits the block has no shell, the recorder never sees the command, and a command block already means something else (it comes back as a shell block).
- **Typed with Enter, opt-in per program, empty by default** (chosen).

## Consequences

- "Typed, never run" is no longer absolute. The glossary's Last command and Auto-resume say where the line is.
- Opting in is a one-word change per program, and the config file (ADR-0007) means it takes effect at the next restore without reinstalling anything.
- An auto-resumed program starts as auto-restore runs, possibly before the Rex app attaches. If programs that query the terminal's colours at startup pick the wrong theme without a client, #61 waits for one, as herdr does.
- An auto-resumed command goes through the block's shell, so it's in the shell's history and the recorder records it as the block's last command.
- A program definition can set `fallback = "none"` (#57): a listed program of its that can't resume then gets the block's last command typed, never run, as an unlisted one would. The rule above still holds for what runs: restore never runs a block's last command.
