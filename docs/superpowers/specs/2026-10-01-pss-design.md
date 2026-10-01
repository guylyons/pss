# pss — a readable `ps` in Rust

## Goal

A Rust reimplementation of `ps` for daily use that is easier to read than
the stock tool: colorized output and a built-in `less`-style pager. It is
a personal tool first; flag compatibility with macOS/BSD `ps` (especially
`ps aux`) matters, exact byte-for-byte output does not.

## Scope

**v1 (this spec)**

- Platform: macOS only. Linux comes later behind the same `ProcessSource` trait.
- Flags: BSD/macOS-compatible subset, plus our own extras.
- Highlighting: resource heat, process state, users, command-line parts.
- Pager: `less`-style snapshot viewer (scroll, page, `/` search, pinned header, `q`).
- Interactive selection (added after v1): a selection bar, `K` to kill the
  selected process, `r` to refresh, `-i` to open the view for short output.
- Extras: `--sort`, pattern search (`pss node`), `--no-pager`, human-readable sizes.

**Deferred**: in-pager sort keys, auto-refresh (live/htop mode), `--tree`,
Linux backend, self/parent marking, zebra striping.

## Architecture

```
cli/      argv → Options (hand-written parser: BSD bundles like `aux` + dash flags + --long)
source/   ProcessSource trait → Vec<Process>
            macos.rs: proc_listallpids + proc_pidinfo + sysctl (the only unsafe code)
select    filter (default / -a / -x / -A / -p / -u / -U / -t / pattern, excludes self) → sort
format/   column keywords → Table of cells; cells carry a semantic Role, not a color
theme     Role → Style (fg, bold, dim, reverse); disabled entirely for --color=never
output/   plain.rs: ANSI or plain text to stdout (truncate to terminal width on a tty)
          pager.rs: ratatui + crossterm viewer
```

`Process` is a plain struct; any field macOS refuses without root is an
`Option` and renders as `-` (dimmed).

Rendering goes through one neutral representation, `Vec<StyledLine>`
(spans of text + optional `Role`). Plain output and the pager both consume it,
so highlighting logic exists once.

## Data source (macOS)

| Data | Call | Without root, other users' processes |
|------|------|-----------------|
| pid, ppid, pgid, uid, ruid, nice, flags, status, tty dev, tpgid, start time, comm | `sysctl(KERN_PROC_ALL)` → `kinfo_proc` (declared by hand; layout asserted at compile time) | ok |
| rss, vsz, cpu times, thread count | `proc_pidinfo(PROC_PIDTASKINFO)` | `None` |
| per-thread run state, cpu usage (for STAT letter and %CPU, as Apple ps does) | `PROC_PIDLISTTHREADS` + `PROC_PIDTHREADINFO` | `None` (letter shows `?`) |
| argv | `sysctl(KERN_PROCARGS2)` | `None` (COMMAND shows `(comm)`) |
| total memory | `sysctl hw.memsize` | ok |
| tty name | `devname(tdev, S_IFCHR)` | ok |

Found while building: `proc_pidinfo(PROC_PIDTBSDINFO)` and `proc_pid_rusage`
are also refused for other users' processes, which is why the base record
comes from `kinfo_proc`. pid 0 (`kernel_task`) is hidden, as in ps.

Gotchas: task CPU times are in Mach absolute-time units on Apple Silicon and
need `mach_timebase_info` conversion. `/bin/ps` is setuid root, which is why
it sees everything; `pss` must not be installed setuid. `sudo pss` works.

## CLI

The first bare argument is a BSD option bundle if every character is a BSD
option letter (`aux`, `ax`, `axww`); other bare arguments are search patterns.
`--` forces everything after it to be patterns.

| Flag | Meaning |
|------|---------|
| (none) | your processes with a controlling terminal; columns `PID TTY TIME CMD` |
| `a` / `-a` | include other users' processes (still terminal-only unless `x`) |
| `x` / `-x` | include processes without a controlling terminal |
| `-A`, `-e` | all processes |
| `u` (bundle) | user format: `USER PID %CPU %MEM VSZ RSS TT STAT STARTED TIME COMMAND` |
| `-f` | `UID PID PPID C STIME TTY TIME CMD` |
| `-o list`, `-O list` | custom columns / add columns after PID; `kw=HEADER` renames |
| `-p pids`, `-u users`, `-U users`, `-t ttys` | select (union); replaces the default filter |
| `-r` / `-m` | sort by CPU / memory, descending |
| `-c` | COMMAND shows only the executable name |
| `-w` / `-ww` | wider / unlimited command column on a tty |
| `--sort [-]key[,...]` | sort by keywords, `-` = descending |
| `PATTERN...` | match against the command line (smart case), implies `-A`, highlights matches, excludes `pss` itself |
| `--color=auto\|always\|never` | default auto; `NO_COLOR` disables |
| `-P`, `--no-pager` | never page; `PSS_PAGER=0` does the same |
| `--human` / `--raw` | sizes as `1.2G` vs KB; default human on a tty, raw when piped |
| `-L` | list keywords |
| `-h`/`--help`, `-V`/`--version` | |

Keywords: `pid ppid pgid uid user ruid ruser %cpu(pcpu,cpu) %mem(pmem,mem)
vsz(vsize) rss tt tty stat(state) start(stime) lstart etime time(cputime)
nice(ni) pri comm ucomm command args flags(f) c nlwp(thcount)`. `cpu` and `mem`
mean %CPU/%MEM (unlike Apple ps, where `cpu` is the scheduler factor).
`comm` is argv[0] and `ucomm` the kernel's short name, as in Apple ps. Lists
split on commas/spaces; `kw=HEADER` renames. Unknown keywords error like `ps`
(`pss: foo: keyword not found`).

## Highlighting

| Role | Style |
|------|-------|
| %CPU | <1 dim, <10 normal, <50 yellow, ≥50 bold red |
| %MEM | <1 dim, <5 normal, <20 yellow, ≥20 bold red |
| RSS | <100M dim, <1G normal, <4G yellow, ≥4G red |
| VSZ, unavailable `-` | dim |
| STAT | R green, S/I dim, T yellow, Z/U bold red; modifier chars dim |
| USER | root red, current user bold green, others a stable hashed color from a palette without red/green; on a terminal names over 16 chars are shortened with `…` |
| COMMAND | dir part of argv[0] dim, basename bold, `-flags` cyan, other args normal |
| pattern match | reverse/yellow highlight |
| header | bold underline |

## Output

- stdout is not a tty → plain text (color only with `--color=always`), no truncation, no pager.
- stdout is a tty → color, truncate lines to terminal width (unless `-ww`), and
  open the pager if the output is taller than the terminal (or `-i` is given)
  and paging isn't disabled.

Pager keys: `j/k/↓/↑` move the selection (the view follows it),
`space/f/PgDn` and `b/PgUp` page, `g/G/Home/End`, `←/→` horizontal scroll
(lines are chopped, like `less -S`), `/` search (selects the match), `n/N`
next/previous match, `q/Esc/Ctrl-C` quit. Header row stays pinned; a status
line shows position, search state and messages.

Killing: `K` asks `Kill PID (name)?` in the status line; `y` sends TERM, `9`
sends KILL, any other key cancels. After a signal the list is collected again
with the same options; the selection stays on the same process, or the same
position if it is gone. Errors (another user's process, already exited) show
in the status line. `pss` refuses to signal itself, since dying would leave
the terminal in the alternate screen. `r` collects the list again on demand.

## Errors

Bad flags/keywords → message on stderr, exit 1 (`ps` behavior). A process that
exits mid-scan is skipped silently. Broken pipe (`pss aux | head`) exits 0 quietly.

## Testing

- Unit: CLI parsing, selection, sorting, every column formatter (time, size,
  start, state), `KERN_PROCARGS2` buffer parsing, theme mapping, pager state
  (scrolling, search) without a terminal.
- Integration: run the binary with `--no-pager --color=never` and check against
  `/bin/ps` for fields that should agree (pid, ppid, user, tt, state letter of our own processes).
