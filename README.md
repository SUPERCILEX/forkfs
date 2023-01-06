# ForkFS

ForkFS allows you to sandbox a process's changes to your file system.

You can think of it as a light-weight container: programs still have access to your real system
(and can therefore jump out of the sandbox), but their disk changes are re-routed to special
directories without changing the real file system.

## Installation

> Note: ForkFS is Linux-only.

### Use prebuilt binaries

Binaries for a number of platforms are available on the
[release page](https://github.com/SUPERCILEX/forkfs/releases/latest).

### Build from source

```console,ignore
$ cargo +nightly install forkfs
```

> To install cargo, follow
> [these instructions](https://doc.rust-lang.org/cargo/getting-started/installation.html).

## Usage

Run a command in the sandbox:

```sh
$ sudo forkfs run -- <your command>
```

All file system changes the command makes will only exist within the sandbox and will not modify
your real file system.

You can also start a bash shell wherein any command you execute has its file operations sandboxed:

```sh
$ sudo -E forkfs run bash
```

> Note: be consistent with your usage of `-E`. Bare `sudo` vs `sudo -E` will likely change the
> forkfs environment, meaning sessions that appear in `sudo` will not appear in `sudo -E` and vice
> versa.

More details:

```console
$ forkfs --help
A sandboxing file system emulator

Under the hood, `ForkFS` creates an `OverlayFS` per session. `ForkFS` must therefore be run as sudo
to create these new mount points.

Note: we make no security claims. Do NOT use this tool with potentially malicious software.

PS: you might also be interested in Firejail: <https://firejail.wordpress.com/>.

Usage: forkfs <COMMAND>

Commands:
  help
          Print this message or the help of the given subcommand(s)
  run
          Run commands inside the sandbox
  sessions
          Manage sessions

Options:
  -h, --help
          Print help (use `-h` for a summary)

  -V, --version
          Print version

$ forkfs sessions --help
Manage sessions

Each session has its own separate view of the file system that is persistent. That is, individual
command invocations build upon each other.

Actives sessions are those that are mounted, while inactive sessions remember the changes that were
made within them, but are not ready to be used.

Note: weird things may happen if the real file system changes after establishing a session. You may
want to delete all sessions to restore clean behavior in such cases.

Usage: forkfs sessions <COMMAND>

Commands:
  list
          List sessions
  stop
          Unmount active sessions
  delete
          Delete sessions
  help
          Print this message or the help of the given subcommand(s)

Options:
  -h, --help
          Print help (use `-h` for a summary)

```
