# ForkFS

ForkFS allows you to sandbox a process's changes to your file system.

## Installation

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
