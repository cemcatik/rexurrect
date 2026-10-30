# rexurrect

Save your Rex sessions and recreate them after the Rex server stops: after a
crash, a beta update that needs a restart, or a reboot. Like tmux-resurrect, for Rex.

rexurrect is a single self-contained binary. It talks to Rex only through the `rex` CLI and
needs nothing else at runtime.

> **Status:** early. `save` and `restore` bring back each session's label, its windows in order,
> each window's split layout and ratios, window and block labels you set, each block's working
> directory, each block's earlier output above its prompt, and each block's last command typed
> at that prompt: the one the zsh recorder recorded or, without one, the program running in
> the block or last seen running there. A block that was running Neovim or Claude Code can come
> back with it resumed. Autosave runs `save` every 15 minutes, and auto-restore runs `restore`
> when the Rex server starts.

## Install

Prebuilt binaries are for macOS, on Apple Silicon and Intel, and for Linux, on x86-64 and ARM64
(static, so they run on any distribution). Pick one way to install.

**With [Homebrew](https://brew.sh)**, on macOS:

```sh
brew tap cemcatik/tap
brew install cemcatik/tap/rexurrect
```

Tap the tap first: Homebrew 6 and later won't install from a tap you haven't tapped yourself.
Installing by the formula's full name then trusts only rexurrect, not everything in the tap.

**With the shell installer**, on macOS or Linux, which downloads the binary for your machine and
puts it in `~/.cargo/bin`, where `cargo install` would:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/cemcatik/rexurrect/releases/latest/download/rexurrect-installer.sh | sh
```

**With cargo**, from source, with a [Rust toolchain](https://rustup.rs) (1.89 or later)
installed:

```sh
cargo install --git https://github.com/cemcatik/rexurrect
```

That builds `rexurrect` and puts it in `~/.cargo/bin`, which rustup adds to your `PATH`.

Each release's binaries carry a GitHub build-provenance attestation. To check that an archive
from a [release](https://github.com/cemcatik/rexurrect/releases) was built by this repository's
release workflow:

```sh
gh attestation verify rexurrect-aarch64-apple-darwin.tar.xz --repo cemcatik/rexurrect
```

rexurrect talks to Rex through the `rex` CLI that ships with Rex: the one on your `PATH` (it is
inside Rex blocks), or else the one a running Rex server runs.

### Set up

Then, from a Rex block (so it sees the server and settings you use):

```sh
rexurrect setup
```

`setup` turns [autosave](#autosave) on, exactly as `rexurrect autosave install` does, and takes
the same `--interval` option. Then it prints the two lines still to add, which it leaves for you
to add (it never edits `~/.zshrc` or Rex's `init.lua`):

1. the [auto-restore](#auto-restore) line for Rex's `init.lua`, with a command that appends it
   there;
2. the line that loads [the zsh recorder](#the-zsh-recorder). Installed with Homebrew, that's a
   `source` line naming the recorder Homebrew installed. Installed another way, `setup` points
   you to the next section instead.

Running `setup` again is safe: it replaces the autosave job (the launchd agent, or on Linux the
systemd timer) and prints the same lines again.

### The zsh recorder

[`rexurrect.plugin.zsh`](rexurrect.plugin.zsh) records each command a block's shell
starts as that block's last command. Load it from `~/.zshrc` in one of these ways.

**Installed with Homebrew**, the recorder is already on disk, at
`$(brew --prefix)/share/rexurrect/rexurrect.plugin.zsh`, a path that stays the same across
`brew upgrade`. `rexurrect setup` prints the line to add to `~/.zshrc`:

```sh
source /opt/homebrew/share/rexurrect/rexurrect.plugin.zsh
```

(`/usr/local` instead of `/opt/homebrew` on an Intel Mac.) With oh-my-zsh, link its directory
in as a custom plugin instead, then add `rexurrect` to `plugins=(...)` in `~/.zshrc`:

```sh
ln -s "$(brew --prefix)/share/rexurrect" "${ZSH_CUSTOM:-$HOME/.oh-my-zsh/custom}/plugins/rexurrect"
```

Installed another way, download it, then load it with a plain `source` or as an oh-my-zsh custom
plugin.

**Plain `source`:**

```sh
mkdir -p ~/.zsh
curl -fsSL -o ~/.zsh/rexurrect.plugin.zsh \
  https://raw.githubusercontent.com/cemcatik/rexurrect/main/rexurrect.plugin.zsh
echo 'source ~/.zsh/rexurrect.plugin.zsh' >> ~/.zshrc
```

**As an oh-my-zsh custom plugin:**

```sh
dir="${ZSH_CUSTOM:-$HOME/.oh-my-zsh/custom}/plugins/rexurrect"
mkdir -p "$dir"
curl -fsSL -o "$dir/rexurrect.plugin.zsh" \
  https://raw.githubusercontent.com/cemcatik/rexurrect/main/rexurrect.plugin.zsh
```

Then add `rexurrect` to `plugins=(...)` in `~/.zshrc`. Cloning this repository into that
directory works too.

New shells pick it up. The recorder:

- does nothing outside Rex (no `REX_BLOCK`), so other terminals and tmux never write anything;
- records only in a block's outermost shell: commands you run in a nested shell (`nix develop`,
  `poetry shell`, a `zsh` started by hand) don't replace the command that started it, and after
  `exec zsh` the shell keeps recording;
- records a command as it starts, so a program still running at save time (`nvim`, a dev
  server, `claude`) counts too;
- ignores multi-line commands, keeping the previous one;
- ignores trivial commands on its ignore list (below), keeping the previous one, so the restored
  prompt shows your last meaningful command;
- tracks the last command itself rather than reading your history, so `SHARE_HISTORY` and
  per-directory history don't confuse it.

#### The ignore list

By default the recorder ignores `ls`, `ll`, `la`, `l`, `cd`, `pushd`, `popd`, `pwd`, `clear`,
`reset`, `exit` and `rexurrect` itself. A command line is ignored when it runs one of these
on its own, with any arguments: `ls -la` and `cd src` are ignored, but `cd src && make` is
recorded. Commands are matched by name as you typed them, so an alias counts under its own
name, or by the last part of the path you typed: `/bin/ls` and `target/debug/rexurrect save`
are ignored too.

The list is the zsh array `REXURRECT_IGNORE`. You don't have to set it. To change it, set it
in `~/.zshrc`:

```sh
# After loading the recorder: add to the defaults.
REXURRECT_IGNORE+=(htop tig)

# Anywhere in ~/.zshrc: replace the defaults (before or after loading the recorder).
REXURRECT_IGNORE=(ls cd clear)

# Ignore nothing but multi-line commands.
REXURRECT_IGNORE=()
```

The recorder fills in the defaults when it loads, unless `REXURRECT_IGNORE` is already set.
So adding to the list only works after loading the recorder; with oh-my-zsh, that means after
`source $ZSH/oh-my-zsh.sh`. The list is read at every command, so a change applies straight
away.

## Usage

```sh
rexurrect save      # snapshot every session on the Rex server
rexurrect restore   # recreate the latest snapshot's sessions that aren't live
rexurrect list      # list the snapshots, newest first
```

- **`save`** records every session: its label, its windows in order, each window's tiled split
  layout (directions and ratios), and each block's working directory and last command. Window
  and block labels are recorded only if you set them; windows and blocks with Rex's default
  labels get fresh default labels on restore, not stale copies like `Window 1a2b3c4d`. Each
  save is a new timestamped snapshot; `latest.json` points at the newest. The newest 10
  snapshots are kept (set `keep` in the [config file](#settings) to keep a different number).
  - Untouched sessions are left out: one window holding one shell, with Rex's default window
    and block labels, still in your home directory, and no last command. Sessions that Rex or
    you create and then abandon therefore don't pile up across restarts. A session's own label
    doesn't count, because Rex's generated ones (`amber-ridge`) look like any other, so a
    session you only renamed is still untouched. A restored session keeps its last command, so
    it doesn't turn untouched just because you haven't run anything in it since.
  - If there's nothing to save (no sessions, or only untouched sessions or sessions with no
    windows left), `save` writes nothing, leaves `latest.json` alone and exits 0, so a server
    with nothing on it never replaces your latest snapshot.
  - If Rex's output doesn't have the shape rexurrect expects (Rex is in beta and its API
    can change) or the snapshot fails validation (every session needs a window, every split
    ratio must be between 0 and 1), `save` writes nothing, exits non-zero and says what didn't
    match.
- **`restore`** recreates each saved session from the latest snapshot, unless that session is
  already live, and prints what it `created`, `replaced`, `skipped` and why anything `failed`.
  One session failing doesn't stop the others; `restore` exits non-zero if any failed. Running it
  twice creates no duplicates. Options, which combine:
  - `--snapshot <id>` restores from an older snapshot, using an ID from `list`;
  - `--session <label>` restores only the saved sessions with that label (all of them, if
    several share it), and fails if the snapshot has none;
  - `--dry-run` (`-n`) prints what would be created and skipped, and creates nothing;
  - `--force` replaces each saved session that's live instead of skipping it: see
    [Sessions Rex carries over](#sessions-rex-carries-over).
- **`list`** shows each snapshot's ID, when it was taken (UTC) and its session labels, newest
  first.

Whatever the snapshot, restore skips a saved session that's still live, even if it has changed
since, so restoring an older snapshot never brings back an older copy of a session you still
have. Kill the live one first, or use `--force`, if that's what you want.

### Sessions Rex carries over

Newer Rex builds bring a server's sessions back by themselves when it starts, with their windows
and layouts but without their scrollback or the programs that ran in them. rexurrect would see
each as still live and skip it, so it **replaces** them instead: it creates the saved session
from the snapshot, exactly as any restore does (scrollback, resumed programs and all), and only
then closes Rex's copy. If creating it fails, Rex's copy stays. Replaced sessions get new
session and block IDs, as every restored session does. See
[ADR-0008](docs/adr/0008-replace-sessions-rex-carried-over.md).

- **Auto-restore** replaces them at the server's start, with no setting: the copies you see in
  the Rex app for the first few seconds are closed then, so anything you type into one before
  that is lost. On a `rex config reload` it never replaces anything.
- **`restore --force`** does the same by hand, for every saved session that's live, whether Rex
  brought it back or not. It combines with `--snapshot`, `--session` (only that label's sessions
  are replaced) and `--dry-run` (it prints `would replace` lines and changes nothing). It prints
  `replaced  <label>  <new session>  (closed <old session>)` for each. Run from inside a session
  it replaces, it closes that one last, once everything else is done and its report is in
  `rexurrect.log`: the terminal you ran it in then disappears.
- **A plain `restore`** still skips them, and says so: `skipped  <label>  (carried over by Rex;
  --force replaces it)`. It doesn't count as restoring the server, so the next save doesn't
  either (next point).
- **`save`** on a freshly started server that still holds Rex's copies saves nothing: it leaves
  `latest.json` alone, exits non-zero and logs that the sessions need replacing, so an autosave
  never replaces your latest snapshot with copies that have lost their scrollback. Autosaves pause
  that way, logging each time, until auto-restore or `restore --force` has replaced them.

rexurrect knows Rex's copy of a session by its blocks: Rex keeps their IDs, so a live block
that's in a snapshot taken before the server started can only have been carried over by Rex.

A block that can't come back as it was still comes back as a shell, so the layout around it
stays intact. A block whose directory no longer exists opens in your home directory. A block
that isn't a shell block, a block on another host, and a window whose blocks have all floated
come back as a plain shell in your home directory. Floating blocks aren't saved.

Each restored block's last command is typed at its prompt but **never run**: press Enter to
resume, edit it, or clear it. Only single-line commands are typed; a block with no last
command gets an empty prompt. A restored block keeps its last command through later saves and
restores until the recorder records something new in it or, without the recorder, until a save
sees a program running in it. A block that was running a program rexurrect can resume (Neovim,
say) gets the program's resume command typed instead: see [Resuming programs](#resuming-programs).

Each restored block also shows its scrollback, the output it held at save time, above the
`Last login:` line and the new prompt, styles included. It wraps to the block's current width
and reflows when you resize the block. A block that was showing a full-screen program (`nvim`,
`less`, `top`, anything on the terminal's alternate screen) gets no scrollback, so you never see
a frozen picture of the program; its command still comes back typed at the prompt.

Some programs draw their interface on the main screen instead, where Rex can't tell it from
earlier output. A block running one of the programs listed in `no_scrollback` in the
[config file](#settings) gets no scrollback either. The list holds program names, matched against
the name the program was run by (`claude`) and the name of its executable. It defaults to
`["claude"]` (Claude Code); set it to replace the default, or to `[]` for no programs:

```toml
no_scrollback = ["claude", "aider"]
```

Autosave reads the same file, so it uses the new list from its next run.

A block whose scrollback is too large for Rex to hand over (Rex caps one response at 1 MB, which a
few thousand lines of coloured output can fill) is saved without it, and the save carries on: it
comes back in its directory with its command typed, but nothing above the prompt. The first
save to find a block's scrollback too large adds a line to `rexurrect.log` naming its session
and block; later saves don't repeat it while it stays too large. Clearing the block's scrollback
(Cmd-K) lets the next save keep it again.

Without the recorder (you don't use zsh, or skipped that step), or in a block whose shell
started before you loaded it, rexurrect falls back to the program running in the block at save
time: `nvim`, a dev server or a nested shell comes back with its full command line typed at the
prompt. Each argument is quoted where zsh needs it (`vim 'my file.txt'`), so the typed command
runs the same program with the same arguments. A program with a line break, tab or other control
character in an argument (`sh -c` with a multi-line script, say) can't be typed, so it doesn't
count; the block keeps whatever last command it had. (Earlier versions read the command line
from `ps`, which turned such characters into `\012`-style escapes; a program remembered that way
no longer carries forward.) The save also remembers that program as the block's last command, so
it still comes back after the program exits and the block sits at a prompt. A block sitting at a
prompt that no save has seen a program in comes back with the command restore typed there, if
restore created it, or else an empty prompt. Running `rexurrect` itself (a `save` you ran there)
never counts as a program. Restored blocks fall back like any other: what restore typed is only
their last command until a save sees a program in them. (This replaces the first design, #6, in
which a restored block never fell back, so without the recorder it kept what restore typed
forever.) Whatever the recorder recorded beats all of this.

Restore recognises sessions by where they came from, not by label: two saved sessions with the
same label both come back, a restored session you rename still counts as live, and a session
you created by hand under a saved label doesn't (see
[ADR-0001](docs/adr/0001-match-sessions-by-lineage-not-label.md)).

For two minutes after a restore (any restore: `restore`, auto-restore, or the one a `save` runs
first) creates a session, a save saves that session's blocks as restore created them: working
directory, last command and scrollback. Their shells may still be starting then: a slow shell
config may still be running, and the scrollback may still be printing, so reading them as they
are would save the startup instead. A block the recorder records a command in during those two
minutes is saved as it is. A session whose new blocks restore couldn't tell apart is saved as it
was in the snapshot it came from. The snapshot's scrollback is kept for those two minutes even if
retention removes the snapshot itself.

After a server restart, the first `save` restores before it saves, unless a `restore` already
ran on that server. A save that runs on a freshly started server (an autosave, say) therefore
can't replace your latest snapshot with a near-empty one. A save on a server already restored
into never restores again, even if you've killed restored sessions since. On a server that still
holds sessions Rex carried over, it neither restores nor saves (see
[Sessions Rex carries over](#sessions-rex-carries-over)). A `--dry-run` or
`--session` restore doesn't count, because it leaves sessions out: a save after it would replace
your latest snapshot without the other sessions. Restoring a whole older snapshot does count.
Only one restore runs at a time, and a `save` that starts during a restore waits for it to
finish.

A `restore` or auto-restore in which every session it tried to create failed (a server that
refuses them, say) restored nothing, so it doesn't count either, and the next save restores
first. If every session fails in the restore a `save` runs first too, that save saves nothing:
it leaves `latest.json` alone, exits non-zero, and logs which snapshot failed to restore. It
does count the server as restored, though, so the next save goes ahead and saves what's live
instead of failing the same way forever. To try that snapshot again, run the command the
message gives, `rexurrect restore --snapshot <id>`, before the next save (an autosave
within 15 minutes) replaces `latest.json`; the snapshot itself stays in `list` until retention
removes it. If only some sessions fail, the save goes ahead and still exits non-zero.

rexurrect never starts a Rex server. If Rex isn't running, `save` and `restore` fail and
say so.

Every failure of `save` or `restore`, including each session that fails to restore, is also
appended to `rexurrect.log` in the state directory, one line per failure starting with a
UTC timestamp. Successful runs write nothing there, with three exceptions: auto-restore logs every
outcome (see [Auto-restore](#auto-restore)), save logs each block whose scrollback it
starts saving without because it's too large, and `restore --force` logs its report before it
closes the session it runs in. Once the log passes 1 MB (an autosave failing
every 15 minutes while Rex is quit gets it there in a few months), it keeps only its newest half.

### Which server

`REX_SERVER` if set (Rex sets it in every block's shell), otherwise the `rex` CLI's default.
Autosave saves the server `autosave install` names (see [Autosave](#autosave)).

rexurrect calls that server through the `rex` on your `PATH`. With no `rex` there, it runs the
`rex` a Rex server of yours runs: the server listening at that endpoint, if its command line
says so, otherwise any (never another user's). It names the endpoint in every call, so
whichever `rex` it runs reaches the same server. Rex moves its CLI when it updates, so this is
how autosave finds one (see
[ADR-0003](docs/adr/0003-reach-rex-through-a-running-servers-executable.md)).

### Where state lives

Snapshots and rexurrect's own bookkeeping live in its state directory:

1. `$REXURRECT_STATE_DIR`, if set;
2. otherwise `$XDG_STATE_HOME/rexurrect`;
3. otherwise `~/.local/state/rexurrect`.

It holds `snapshots/`, `latest.json` and `lineage.json`, the lineage map, which remembers which
live session restore created from which saved one. Deleting `lineage.json` makes the next
`restore` recreate sessions that are already live. `restored-instances` names the server
instances restore has run in (the 32 most recent), and `restore.lock` makes restores take turns.
`starting-sessions.json` holds what restore created each session with, for saves in its first
two minutes.
`recorder/` holds one file per block with the last command the recorder recorded, rewritten
each time the shell's prompt comes back; only recorders write there. `default-commands.json`
holds each block's last command when there's no recorder file: the last program a save saw
running in it, else the command restore typed there.
`resume/` holds one file per block with the resume state its program reported through
`rexurrect report` (see [Resuming programs](#resuming-programs)); only `report` writes there, and
`report-failures.json` remembers which of its failures it has logged.
`too-large-scrollback.json` names the blocks the last save saved without scrollback because it
was too large, so each is logged once rather than at every save. Each save that writes a snapshot then removes the `recorder/` and `resume/` files and
`default-commands.json` entries of blocks that no longer exist, and the `lineage.json` and `starting-sessions.json`
entries of sessions that no longer exist, so the directory doesn't grow forever. A save that
fails or writes nothing removes nothing. `scrollback/` holds each snapshot's block scrollback,
which goes when its snapshot goes (or two minutes after a restore from it, if that's later); a
block with 50,000 lines of output takes about 340 KB per snapshot. `rexurrect.log` is the log of
failures, auto-restore outcomes, blocks saved without scrollback because it was too large, and
warnings about [settings that moved](#moving-from-environment-variables).
The recorder finds the state directory the same way, so set `REXURRECT_STATE_DIR` (if you
use it) where both your shells and rexurrect see it.
Autosave and auto-restore run outside your shells, so `autosave install` and
`auto-restore --print-hook` pin the state directory they resolve.

Snapshots, scrollback, recorder files, `default-commands.json` and `starting-sessions.json` hold
commands, and commands can hold secrets, so the state directory is readable by you only: `save` and `restore` make it `0700` if it isn't.
rexurrect writes its own files with mode `0600`, and the recorder keeps its files in a
`0700` directory. Every file except the log is written atomically (a temporary file, then a
rename), so a crash never leaves a partial snapshot or record behind; the log is appended to.
A temporary file that a crash does leave behind is removed by the next save that writes a
snapshot, once it has gone unchanged for an hour.

## Resuming programs

A block that was running a program when it was saved can come back with the program **resumed**,
not just started again: Neovim with its buffers, splits and tab pages, Claude Code with the
conversation you were in. rexurrect knows how to
resume a program through its **program definition**. At each save, a block whose foreground
program matches a definition (by the name it was run by or its executable's name, as
`no_scrollback` matches, or, for a program an interpreter runs, such as an npm-installed agent
that runs as `node`, by its script's file name) is saved as resumable. At restore, the definition builds the program's
resume command, which is typed at the block's prompt in place of its last command, and **never
run**: press Enter to resume. Programs you list in `auto_resume` come back running instead (see
[Auto-resume](#auto-resume)).

Only the program running at the save resumes. A block where you had quit Neovim before the save
comes back with its last command, even if that command is `nvim init.lua`. So does one where you
quit it after the save but before the server stopped, if the block's shell loads the zsh
recorder: the recorder notes each time the shell's prompt comes back, so restore can tell, and
the block gets the last command its recorder had at the stop, typed, never run, even for a
program in `auto_resume`. Without a recorder, that block still resumes. A block that can't
resume (see below) comes back with its last command too, as it always did. A resume command you
don't run carries forward like any command restore typed: the block's shell is at its prompt at
the next save, so the block isn't resumable any more, and it keeps that command as its last
command until you run something.

Snapshots taken before rexurrect resumed programs restore as they always did.

Built in:

| Program | Resumes with | When |
| --- | --- | --- |
| Neovim (`nvim`) | `nvim -S` | a `Session.vim` is in the block's working directory |
| Claude Code (`claude`) | `claude --resume <conversation ID>` | its hook has reported the conversation |
| Codex (`codex`) | `codex resume <conversation ID>` | its hook has reported the conversation |

Some programs need to tell rexurrect something for it to resume them: an agent its conversation's
ID, say. That's the program's **resume state**, and the program reports it with
`rexurrect report <program>`, from a hook in its own config. It's only ever a value, never a
command: rexurrect checks it's the kind of value the program's definition expects (an ID is
letters, digits, `-`, `_`, `.` and `:`, and never starts with `-`) and builds the command
itself, so a hook can't make restore type anything else. `report` never prints and always
exits 0, so nothing it does shows up in, or holds up, the program that ran it; what it can't do
goes to `rexurrect.log`, once per cause. What it stores is private to rexurrect, in the state
directory, and a save forgets the blocks that are gone.

### Neovim

Neovim resumes from its **Neovim session** (its own record of its buffers, windows, tab pages and
directory, nothing to do with a Rex session), following [vim-obsession]'s convention, which
tmux-resurrect's `session` strategy uses: a `Session.vim` in the working directory. A block
that was running `nvim` comes back with `nvim -S` typed if there's a `Session.vim` in its
working directory at restore, and with its last command otherwise.

rexurrect never writes Neovim sessions: your session manager does. With vim-obsession, run
`:Obsess` once in a directory and it keeps `Session.vim` up to date from then on. Neovim saves
on exit when its terminal goes away, so a session manager that saves on exit has saved by the
time the Rex server is gone.

With another session manager, point the built-in definition at it (see
[Your own definitions](#your-own-definitions)):

```toml
# persistence.nvim, which LazyVim uses: it keeps its sessions elsewhere, and load() is safe
# without one.
[programs.nvim]
resume = "nvim -c 'lua require(\"persistence\").load()'"
requires_file = ""

# resession, with a session per directory:
[programs.nvim]
resume = "nvim -c 'lua require(\"resession\").load(vim.fn.getcwd(), { dir = \"dirsession\", silence_errors = true })'"
requires_file = ""

# auto-session:
[programs.nvim]
resume = "nvim -c SessionRestore"
requires_file = ""
```

(Pick one: a file can have only one `[programs.nvim]`.)

[vim-obsession]: https://github.com/tpope/vim-obsession

### Claude Code

Claude Code reports its conversation from a `SessionStart` hook, which it runs whenever a
conversation starts, resumes, or is cleared or compacted, so the block always holds the
conversation on screen. Subagents don't change it. rexurrect never edits Claude Code's config;
print the hook and add it to `~/.claude/settings.json` yourself:

```sh
rexurrect report claude --print-hook
```

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "command": "REXURRECT_STATE_DIR=/Users/you/.local/state/rexurrect REXURRECT_CONFIG=/Users/you/.config/rexurrect/config.toml /Users/you/.cargo/bin/rexurrect report claude",
            "type": "command"
          }
        ]
      }
    ]
  }
}
```

It names rexurrect by its full path and pins the state directory and config file, as
`auto-restore --print-hook` does, so print it again if you move any of them. If your
`settings.json` already has hooks, add the `SessionStart` entry beside them.

From then on, a block that was running `claude` comes back with `claude --resume <its
conversation>` typed. The command drops any flags `claude` was started with. A block whose
`claude` hasn't reported a conversation (it started before you added the hook) comes back with
its last command. If two blocks were in one conversation, only the first, in layout order, gets
the resume command, so two Claude Codes never write to one conversation.

A resumed agent reopens its conversation and waits at its prompt. It doesn't carry on by
itself: a reply it was in the middle of when the server stopped is gone, so ask it to continue.

### Codex

Codex reports its conversation (Codex calls it a thread) from a `SessionStart` hook, which, as
Claude Code's, runs whenever a conversation starts, resumes, or is cleared, compacted or forked.
A Codex started from inside another Codex (by its agent, say) doesn't change the block: it
inherits the outer one's `CODEX_THREAD_ID`, and its reports for any other conversation are
ignored, as herdr does. Print the
hook and the line that turns Codex's hooks on:

```sh
rexurrect report codex --print-hook
```

```
# In ~/.codex/hooks.json:
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "command": "REXURRECT_STATE_DIR=/Users/you/.local/state/rexurrect REXURRECT_CONFIG=/Users/you/.config/rexurrect/config.toml /Users/you/.cargo/bin/rexurrect report codex",
            "type": "command"
          }
        ]
      }
    ]
  }
}

# In ~/.codex/config.toml:
[features]
hooks = true
```

Add the `SessionStart` entry to `hooks.json` beside any hooks it has, and `hooks = true` to the
`[features]` table of `config.toml` (in `$CODEX_HOME` instead of `~/.codex`, if you set it).
Codex runs a hook only once you trust it: the next time it starts, it says the hook needs
review, and you trust it there. It asks again whenever the hook's command changes, as it does
when you print it again after moving rexurrect.
From then on, a block that was running `codex` comes back with `codex resume <its
conversation>` typed. Codex runs the hook with its first turn, not when it opens, so a Codex
you hadn't sent anything yet (which has no conversation to resume) comes back with its last
command. Codex installed with npm runs as `node`, and is matched by its script's name, `codex`.

### Auto-resume

By default, restore never runs anything: every resume command is typed for you to run. Programs
you list in `auto_resume` in the [config file](#settings) **auto-resume**: restore types their
resume command followed by Enter, so they come back running.

```toml
auto_resume = ["claude", "nvim"]
```

The list is empty by default, so nothing runs that didn't before until you add a program to it
(see [ADR-0006](docs/adr/0006-auto-resume-runs-commands-opt-in.md) for why). It takes effect at
the next restore, with nothing to reinstall.

An auto-resumed program runs in the block's shell, as if you had pressed Enter: it's in your
shell's history, the recorder records it as the block's last command, and when you quit it
you're at your prompt in the block's directory.

A listed program that can't resume (Claude Code that reported no conversation, Neovim without
a `Session.vim`, or a second block in a conversation another block resumes) still comes back
running: restore runs its own command line as it was at the save (`nvim src/main.rs`,
`claude --model opus`), so it comes back without its old state. Never the block's last command,
which may only have started it: a block where `make edit` or `git commit` opened Neovim never
reruns `make edit` or `git commit`. A program whose command line couldn't be read at the save
(one with a line break in an argument, say) gets its last command typed instead.

An auto-resumed program starts as soon as restore creates its block, which, under auto-restore,
may be before the Rex app has attached to the server.

A program listed in `auto_resume` whose definition sets `fallback = "none"` comes back with its
last command typed instead, when it can't resume.

### Your own definitions

Change a built-in definition, or add one for a program rexurrect doesn't know, in the
[config file](#settings), with no code from rexurrect. A `[programs.<name>]` table holds a
definition's fields:

| Field | What it is | For a new program |
| --- | --- | --- |
| `match` | The program names it matches: the name it was run by, its executable's, or, for an interpreter (`node`, `bun`, `python`, `python3`, `ruby`, `perl`), its script's file name | `[<name>]` |
| `state` | The resume state the program reports: `"none"`, `"id"` or `"path"` (absolute) | `"none"` |
| `resume` | The resume command; `{state}` stands for the reported value, quoted where zsh needs it | required |
| `requires_file` | A file that must be in the block's working directory for it to resume | none |
| `fallback` | What an auto-resumed program that can't resume runs: `"command"`, its own command line, or `"none"`, its last command typed | `"command"` |
| `enabled` | `false` turns the definition off | `true` |

A table named like a built-in (`nvim`, `claude`, `codex`) overrides it field by field: the fields you set
replace the built-in's, and the rest keep the built-in's. `requires_file = ""` removes the
built-in's requirement, and `enabled = false` turns it off, so its blocks come back with their
last command. `resume` uses `{state}` exactly when `state` isn't `"none"`. Each definition's
name goes in `auto_resume` and after `rexurrect report`. If a program matches several
definitions, yours come before the built-ins, in name order.

A program of your own reports its state with `rexurrect report <name> <value>`, from its own
hook or plugin, run in the block (where `REX_BLOCK` is set). `report` keeps the value only if
it's the kind `state` says: say, a REPL that prints how to attach to it again:

```toml
auto_resume = ["myrepl"]

[programs.myrepl]
state = "id"
resume = "myrepl --attach {state}"
```

```sh
# In myrepl's startup hook:
rexurrect report myrepl "$MYREPL_SESSION_ID"
```

`rexurrect report myrepl --print-hook` prints that command with rexurrect's full path, and the
state directory and config file pinned.

To keep a flag Claude Code was started with, add it to the built-in's command:

```toml
[programs.claude]
resume = "claude --resume {state} --dangerously-skip-permissions"
```

## Settings

rexurrect reads its settings from one file, `~/.config/rexurrect/config.toml`, or
`$XDG_CONFIG_HOME/rexurrect/config.toml` if `XDG_CONFIG_HOME` is an absolute path. Set
`REXURRECT_CONFIG` to use another file. You don't need one: with no file, every setting has its
default. This is every setting, each set to its default:

```toml
# How many snapshots save keeps, newest first: a whole number, at least 1.
keep = 10
# The programs whose blocks save keeps no scrollback for (see Usage); [] for none.
no_scrollback = ["claude"]
# How long auto-restore waits for the Rex server to be ready, in seconds.
ready_timeout = 60
# How long auto-restore looks for the app session after restoring, in seconds.
app_session_watch = 30
# The programs that come back running, not just typed (see Resuming programs); [] for none.
auto_resume = []
```

`[programs.<name>]` tables change and add program definitions: see
[Your own definitions](#your-own-definitions).

Manual runs, autosave and auto-restore all read the same file, at every run, so an edit takes
effect the next time any of them runs, with nothing to reinstall. Autosave and auto-restore run
outside your shells, so `autosave install` and `auto-restore --print-hook` pin the file's path
as they find it, the way they pin the state directory: its path, never its settings. Run them
again after moving the file or changing `REXURRECT_CONFIG` or `XDG_CONFIG_HOME`.

A file rexurrect can't use (one it can't read, one that isn't TOML, a key it doesn't know, a
value of the wrong kind) fails the run with an error naming the file and the key. Under autosave
and auto-restore, the error goes to `rexurrect.log` like any other failure.

Two settings stay environment variables, because the zsh recorder reads them and it can't read
TOML:

- `REXURRECT_STATE_DIR` (see [Where state lives](#where-state-lives)): the recorder finds the
  state directory itself, at every command and without starting another program, and it and
  rexurrect must agree on it;
- `REXURRECT_IGNORE` (see [The ignore list](#the-ignore-list)): only the recorder reads it, so it
  belongs in `~/.zshrc`.

`REX_SERVER` and `REXURRECT_CONFIG` are variables too, and so are the overrides for testing under
[Development](#development).

### Moving from environment variables

Earlier versions of rexurrect read these settings from environment variables. It no longer reads
them: move each one you set into the file.

| Was                                      | Now                                   |
| ---------------------------------------- | ------------------------------------- |
| `REXURRECT_KEEP=30`                      | `keep = 30`                           |
| `REXURRECT_NO_SCROLLBACK="claude aider"` | `no_scrollback = ["claude", "aider"]` |
| `REXURRECT_READY_TIMEOUT=120`            | `ready_timeout = 120`                 |
| `REXURRECT_APP_SESSION_WATCH=10`         | `app_session_watch = 10`              |

While one is still set, every run warns, naming the line to put in the file instead:

```
rexurrect: REXURRECT_KEEP is no longer read: set keep = 30 in /Users/you/.config/rexurrect/config.toml
```

On a terminal the warning goes to stderr; under autosave and auto-restore, to `rexurrect.log`.
Once you've moved them, remove them from `~/.zshrc` (and from the auto-restore line's `env`, if
you added them there), then:

- run `autosave install` once: an agent installed by an earlier rexurrect carries the variables
  it copied when you installed it, and warns at every autosave until you reinstall it;
- replace the auto-restore line in `init.lua` with the one `auto-restore --print-hook` prints
  now, which pins the config file's path (see [Auto-restore](#auto-restore)).

## Autosave

```sh
rexurrect autosave install                 # save every 15 minutes
rexurrect autosave install --interval 5    # or every 5
rexurrect autosave uninstall
```

`autosave install` puts a launchd user agent in `~/Library/LaunchAgents`
(`io.github.cemcatik.rexurrect.autosave.plist`) and loads it, so `rexurrect save` runs
every 15 minutes, or every `--interval` minutes, from now on and after every login. Running it
again replaces the agent, so there's only ever one. macOS may show a one-time "Background Items
Added" notification. On Linux it installs a systemd user timer instead (see
[Autosave on Linux](#autosave-on-linux)).

Autosave is silent: its output is discarded, and a failure (Rex not running, an API error, a
snapshot that fails validation) only adds a line to `rexurrect.log` in the state directory.
While Rex isn't running, each autosave fails and logs it; it never starts a server. On a freshly
started server, autosave restores before it saves, like any `save`.

launchd starts the agent with almost no environment: no `REX_SERVER`, a bare `PATH`, nothing
from your shell config. So `autosave install` writes into the agent what `save` needs, as it
finds it when you run it, and prints the server, state directory and config file it chose:

- **the server**: `REX_SERVER` if set, so run it from a Rex block to save the server that block
  is on. Otherwise the Rex app's server,
  `unix://$HOME/Library/Application Support/rex/server.sock`, even if `XDG_RUNTIME_DIR` would
  point the `rex` CLI elsewhere. Autosave always names that server explicitly, so it can't end
  up on another one;
- **the state directory**, resolved as above, so autosave shares snapshots with your manual
  runs and the recorder;
- **the config file's path**, resolved as in [Settings](#settings). Its path, not its settings:
  each autosave reads the file as it is then, so an edit reaches the next autosave.

Run `autosave install` again after changing any of these, or after moving `rexurrect`.

The agent's `PATH` is only the system directories, so it has no `rex` on it: Rex moves its CLI
when it updates, and a directory pinned at install would be gone after the next update. Each
autosave runs the `rex` a running Rex server runs instead (see [Which server](#which-server)).
While Rex isn't running there's no server to save, and the autosave fails and logs it as above.

**After upgrading rexurrect**, run `autosave install` once. An agent installed by an earlier
rexurrect has the directory of the `rex` that was on your `PATH` back then pinned on its
`PATH`, and keeps using that `rex` for as long as the directory exists, even after Rex has
updated past it.

`autosave uninstall` unloads the agent and removes its plist, then tells you how to remove the
zsh recorder and the auto-restore line in `init.lua`. Your snapshots stay in the state
directory.

To check that autosave works, run `launchctl print gui/$(id -u)/io.github.cemcatik.rexurrect.autosave`
(its `last exit code` should be 0 after the first run) and `rexurrect list` (a new snapshot
every interval), and look in `rexurrect.log` for failures.

## Linux

rexurrect works against a Rex server on Linux too. So far it has been tried on one setup:
Ubuntu 20.04 with systemd 245, Rex installed from its `.deb` (one binary, `/usr/bin/rex`), and
each account's server started by Rex's login server (`rex-login-server.service`), reached
from the Rex app on a Mac. The account's shell there is bash. Other distributions, init
systems and setups will be added here as they're tried and reported.

**Install** as above, with the shell installer or cargo (Homebrew on Linux isn't supported), then
[set up](#set-up) with `rexurrect setup`, which installs the systemd timer described
[below](#autosave-on-linux). The recorder is in each release archive, but neither install puts
it where `setup` looks, so for a zsh account `setup` points you to
[the zsh recorder](#the-zsh-recorder) instead of printing a `source` line. Last commands come
from the program running in each block (see [Usage](#usage)): the recorder is zsh-only, so a
bash block's last command is the program that was in its foreground, not the last command typed
at its prompt.

`save`, `restore` and auto-restore work as on macOS. With no `REX_SERVER` set (outside Rex,
over SSH, say), rexurrect uses `rex`'s default endpoint, `$XDG_RUNTIME_DIR/rex/server.sock`,
which is where the login server's per-account server listens. With no `rex` on `PATH` (a
`--tarball` install in `/usr/local/bin`, or the autosave timer), it runs the `rex` your
running server runs, read from `/proc/<pid>/exe` (see [Which server](#which-server)).

### When `rex` is (R)?ex

Debian and Ubuntu ship an unrelated package also named `rex`: (R)?ex, a Perl automation tool.
Rex's installer pins its own repository above it, but `unattended-upgrade` ignores that pin
(Rex's repository isn't one of its allowed origins), so on a machine with unattended upgrades
and universe enabled it can replace Rex with (R)?ex as an ordinary upgrade. Servers already
running keep working, but every `save` and `restore`, autosave's included, then fails, naming
the `rex` it ran as (R)?ex, not Rex's CLI.

To get Rex back, run Rex's install script again, as
[Rex's installation docs](https://www.superlogical.com/rex/docs/linux/installation#install) say.
Or, with apt alone, name Rex's version, since apt now sees (R)?ex as the newer one:
`sudo apt-get install --allow-downgrades rex=<version>`, with a version `apt-cache madison rex`
lists from Rex's repository. To keep it from happening again, have `unattended-upgrade` leave the
`rex` package alone, in a file of its own such as `/etc/apt/apt.conf.d/51rex`:

```text
Unattended-Upgrade::Package-Blacklist { "rex$"; };
```

Entries are regular expressions matched from the start of a package's name, so without the `$`
it would leave every package whose name starts with `rex` alone too. This costs nothing:
`unattended-upgrade` never installed Rex's own updates, and `apt upgrade` still follows Rex's pin.

Other distributions may package (R)?ex, or some other tool, as `rex` too, and their automatic
updates may swap it in the same way. rexurrect recognises (R)?ex wherever it comes from, by what
`rex -v` prints. Any other tool named `rex` just fails with its own error, so if `save` fails
with output that doesn't look like Rex's, check what `rex -v` prints.

### Autosave on Linux

`autosave install` writes two systemd user units to `$XDG_CONFIG_HOME/systemd/user` (else
`~/.config/systemd/user`): `rexurrect-autosave.service`, which runs `rexurrect save` once
with the environment described in [Autosave](#autosave), and `rexurrect-autosave.timer`, which
runs it every 15 minutes (or `--interval` minutes), the first time one interval after it
starts. Install then runs `systemctl --user daemon-reload` and enables and (re)starts the
timer, so it runs from now on and whenever your systemd user manager runs. Installing again
replaces both units. `autosave uninstall` stops and disables the timer and removes both units.

With no `REX_SERVER` set at install, autosave saves the server at `rex`'s default endpoint,
`unix://$XDG_RUNTIME_DIR/rex/server.sock` as install finds it (there's no Rex app to fall
back to). If `XDG_RUNTIME_DIR` isn't set either (under `su`, say), install fails and asks you
to set `REX_SERVER`. The service's output is discarded, as on macOS; failures go to `rexurrect.log`.

To check that autosave works:

```sh
systemctl --user list-timers rexurrect-autosave.timer   # when it runs next, and last ran
systemctl --user status rexurrect-autosave.service       # how the last run exited
journalctl --user -u rexurrect-autosave.service          # each run's start and finish
rexurrect list                                           # a new snapshot every interval
```

**Linger.** Your systemd user manager, and with it the timer, runs only while you have a login
session, unless linger is on for your account. rexurrect never checks or changes linger.

- **Rex's login server** opens a login session for your account for as long as it runs your
  server, so the timer runs exactly while there's a server to save. No linger needed.
- **A server you start by hand** with `rex server start` over SSH outlives your SSH login, but
  the timer stops when you log out. To keep autosaving it, have linger on:
  `loginctl enable-linger $USER` (it may need an admin).

**The shared server** (`rex.service`, listening at `/run/rex/server.sock` and running as the
`rex` account) hasn't been tried. It should work if rexurrect runs as the `rex` account, since
its blocks already do: install rexurrect for that account, put the auto-restore line in that
account's `init.lua`, run `autosave install` as it with `REX_SERVER=unix:///run/rex/server.sock`,
and have an admin enable linger for `rex` (`loginctl enable-linger rex`), since nobody logs in
as it.

**Servers that stop when empty.** The login server starts each server with `--exit-empty`:
once no session and no client connection is left, the server stops, and the next connection
starts a new one. So with no client connected, closing your last session stops the server, and
the next connection's auto-restore brings back the latest snapshot's sessions, including the
ones you just closed. That's what happens after any restart on macOS too: a save from an
empty server never replaces the latest snapshot. Servers on Linux just restart more often.

## Auto-restore

Auto-restore runs `restore` by itself once each time the Rex server starts, so your sessions come
back after a crash, an update or a reboot without you doing anything. It's one line in Rex's
`init.lua`, which is `~/.config/rex/init.lua` unless you've moved it (`rex config check` shows
where yours is).

**To turn it on**, add the line. `auto-restore --print-hook` prints it; run this from a shell
that sees your usual environment (a Rex block, say):

```sh
mkdir -p ~/.config/rex &&
  rexurrect auto-restore --print-hook >> ~/.config/rex/init.lua
```

The line reads

```lua
do local r = rex.exec({ [[/Users/you/.cargo/bin/rexurrect]], "auto-restore" }, { env = { REXURRECT_STATE_DIR = [[/Users/you/.local/state/rexurrect]], REXURRECT_CONFIG = [[/Users/you/.config/rexurrect/config.toml]] } }) if r then io.stderr:write(r.stderr) end end
```

The server runs it in its own environment, not your shell's: the Rex app starts its server with
a bare `PATH` and no `XDG_*` variables. So the line names `rexurrect` by its full path, and
pins the state directory and the config file as they were resolved when you printed it (see
[Where state lives](#where-state-lives) and [Settings](#settings)), so auto-restore uses the
same snapshots as your shells and their recorder, and the settings you edit. It pins the config
file's path, not its settings: an edit takes effect at the server's next start. Print the line
again, replacing the old one, after moving `rexurrect` or changing `REXURRECT_STATE_DIR`,
`XDG_STATE_HOME`, `REXURRECT_CONFIG` or `XDG_CONFIG_HOME`.

**If your `init.lua` has an `os.execute(...)` line** that runs `rexurrect auto-restore`, the
line `--print-hook` printed before Rex build 1203, replace it with the line it prints now. Rex
1203 made `os.execute` an error, so the server stops running `init.lua` at that line: nothing is
restored, and nothing after it in `init.lua` runs either.

**To turn it off**, delete that line. `autosave uninstall` reminds you to.

The server runs `init.lua` when it starts, and again on every `rex config reload`. The line starts
`rexurrect auto-restore`, which returns at once, because the server can't answer anything
until `init.lua` has finished, and carries on in the background:

- It waits for the server to be ready, then restores the latest snapshot into it the way
  `restore` does. At the server's start, saved sessions that are live (the ones Rex carried
  over, without their scrollback or programs) are replaced, as `restore --force` replaces them
  (see [Sessions Rex carries over](#sessions-rex-carries-over)). On a config
  reload they're skipped, so nothing is duplicated.
- It restores once per server instance. On a config reload, or on a server that `restore` or a
  `save` already restored into (see [Usage](#usage)), it restores nothing, so sessions you killed
  stay gone. Adding the line and running `rex config reload` restores into the running server
  only if nothing has restored into it yet.
- It restores into the server that ran `init.lua`, even one started from inside another server's
  block, whose `REX_SERVER` that server inherits. It reads the server's endpoint from the server
  process's command line, uses it only once the server there reports the right process ID, and
  then names it explicitly in every call. It runs the `rex` that the server itself runs.
- It's silent. Its output is discarded, and each run adds one line to `rexurrect.log` saying
  which server it restored into and what it created, replaced and skipped, and the app session
  it closed (next point), that there was nothing to do, or why it gave up, plus a line for each
  failure.
  A run that created sessions adds its line once it stops looking for the app session, up to
  30 seconds after restoring. If the server isn't ready within 60 seconds, it gives up and
  logs that. Set `ready_timeout` (seconds) in the [config file](#settings) to wait longer. A
  failure before it carries on in the background (a config file it can't use, say) goes to the
  server's own log too.
- The Rex app creates a session of its own when it finds the server with no sessions, even
  while auto-restore is creating yours, so every restart would leave this app session, untouched,
  beside them. Once auto-restore has created sessions, it closes that app session: it looks for 30
  seconds after restoring, and closes the first live session it finds that restore didn't
  create and that's untouched (see [Usage](#usage)), the earliest if there are several
  (`app_session_watch` in the [config file](#settings) sets how long it looks). It
  closes at most one per server instance, and names it in `rexurrect.log`. A session you open
  yourself in those 30 seconds and haven't used yet looks just like the app's, so if the app's
  doesn't turn up first, yours is closed instead; nothing in it was worth restoring. An app
  session that turns up after the 30 seconds is left alone, and while it's untouched `save`
  never saves it. Only auto-restore closes it: `restore` and the restore a `save` runs first
  never close an app session. See
  [ADR-0004](docs/adr/0004-close-the-app-session-after-auto-restore.md).

`rex config check` runs `init.lua` too. There auto-restore says it wasn't started by a Rex server
and does nothing. Any other Rex server that reads the same `init.lua` (one you start by hand with
`rex server run` and no `--config`, say) gets your sessions restored into it too. That doesn't
make your own server look unrestored: rexurrect remembers every recent server instance it
restored into, not just the last.

If auto-restore can't tell which endpoint is the server's (one containing a space that isn't
percent-encoded; Rex's own endpoints are encoded), it gives up and logs that rather than restore
into any other server.

## Uninstalling

1. **Turn autosave off first**, while rexurrect is still installed:

   ```sh
   rexurrect autosave uninstall
   ```

   A Homebrew formula has no uninstall hook, so `brew uninstall` can't remove the autosave agent.
   An agent left behind runs a binary that's gone, every 15 minutes, and since its output is
   discarded, it fails without a trace.

2. **Uninstall the binary**: `brew uninstall rexurrect`, or `cargo uninstall rexurrect`, or,
   for the shell installer, `rm ~/.cargo/bin/rexurrect`.

3. **Remove the two lines** `setup` had you add: the auto-restore line from Rex's `init.lua`, and
   the line that loads the recorder from `~/.zshrc` (with oh-my-zsh, `rexurrect` in
   `plugins=(...)` and its directory or link under `$ZSH_CUSTOM/plugins`). Then open new shells.

Your snapshots stay in the [state directory](#where-state-lives); delete it if you don't need
them.

## Switching install method

Autosave and the auto-restore line name the `rexurrect` they were set up with, by its full path:
`/opt/homebrew/bin/rexurrect` from Homebrew, `~/.cargo/bin/rexurrect` from cargo or the shell
installer. (Those two share a path, so switching between them needs nothing.) After switching
between Homebrew and either of them, from a new shell where `rexurrect` is the one you keep:

1. run `rexurrect setup` again, which replaces the autosave agent;
2. replace the auto-restore line in `init.lua` with the one it prints;
3. if you load the recorder from Homebrew's copy and are leaving Homebrew, load a downloaded one
   instead (see [the zsh recorder](#the-zsh-recorder)), or switch to Homebrew's if you're moving
   to it;
4. uninstall the copy you're leaving (step 2 of [Uninstalling](#uninstalling)).

## Development

CI runs these checks on every pull request and every push to main. Run them before pushing:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --document-private-items --locked
cargo test --locked                      # CI: only `--bins`, and `--tests --no-run`
zsh -n rexurrect.plugin.zsh
cargo deny --locked check                # licenses, advisories, bans, sources
cargo +1.89 build --all-targets --locked # the minimum Rust version, rust-version
typos
zizmor .github                           # the workflows, dist's included
```

**CI doesn't run the integration tests**, only compiles them. They run the built binary against
throwaway Rex servers, and Rex is a private beta that CI can't install, so CI never downloads,
links to or references it. A green check means the integration tests compile, not that they
pass: run `cargo test` locally.

The integration tests run the built binary against throwaway Rex servers, so they need the
`rex` CLI on `PATH` (or its path in `REX_BIN`). Each test starts its own server under
`/tmp/rxt.*` with its own state directory, config file and minimal zsh config, and stops it
afterwards. They never touch your real Rex server, sessions or snapshots. The shared harness is
in `tests/common/`.

Autosave's tests never touch the real launchd or systemd either: `launchctl` and `systemctl`
are found on `PATH`, so they put a stand-in first, and `REXURRECT_LAUNCH_AGENTS_DIR` replaces
`~/Library/LaunchAgents`, and `REXURRECT_SYSTEMD_USER_DIR` the systemd user unit directory.
`REXURRECT_SCHEDULER` (`launchd` or `systemd`) picks the scheduler instead of the platform, so
the systemd tests run on macOS too; the launchd tests run on macOS only.
To try autosave on the real launchd or systemd without touching your own job, also set
`REXURRECT_AUTOSAVE_LABEL` to a scratch label (for both `install` and `uninstall`); with
systemd it names the units.

Code for Linux only (reading `/proc`) isn't compiled on macOS, so CI also runs clippy, the unit
tests and the integration tests' compile on Linux. To type-check it on macOS:

```sh
rustup target add x86_64-unknown-linux-musl
cargo clippy --target x86_64-unknown-linux-musl --all-targets -- -D warnings
```

To build for Linux (a static binary; every dependency is pure Rust), add
`CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld` and run
`cargo build --release --target x86_64-unknown-linux-musl`.

Each test's config file (`REXURRECT_CONFIG`) sets `app_session_watch = 1`, so a test waiting
for auto-restore's entry in the log doesn't wait half a minute. A test that sets other settings
writes its own file in its place.

Terms (session, window, block, label, lineage, ...) follow [GLOSSARY.md](GLOSSARY.md).

## Releasing

```sh
scripts/release.sh 0.2.0
```

The script refuses to release unless the working tree is clean, the tag `v0.2.0` doesn't exist
yet, and formatting, clippy and the **full** `cargo test` pass, integration tests included, so it
needs Rex installed. It checks all of that before changing anything. Then it bumps the version,
commits `chore: release v0.2.0`, tags it, and pushes the commit and the tag together.

The tag starts the release workflow, which [dist](https://axodotdev.github.io/cargo-dist) generates
from `dist-workspace.toml`: it builds the binaries for macOS (Apple Silicon and Intel) and Linux
(x86-64 and ARM64), attests them, publishes the GitHub release with the shell installer, and
updates the `rexurrect` formula in [cemcatik/homebrew-tap](https://github.com/cemcatik/homebrew-tap).
A prerelease version (`0.2.0-rc.1`) publishes a GitHub release and leaves the formula alone.
Every pull request runs dist's plan, so a broken release config shows up before a tag does.

Change the release workflow through `dist-workspace.toml` and `dist generate`, never by hand: a
hand-edited workflow makes dist's plan fail. That includes the actions it uses, pinned to commit
SHAs in `[dist.github-action-commits]`; Dependabot leaves that workflow alone.

The repository needs, set by hand: a `HOMEBREW_TAP_TOKEN` secret (a fine-grained token with
contents read and write on `cemcatik/homebrew-tap` only), immutable releases, and the policy that
requires actions pinned to full commit SHAs.
