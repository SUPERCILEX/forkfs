# ForkFS

ForkFS allows you to isolate a process's changes to your files, sandboxing those changes into
another directory.

## Installation

TODO

## Usage

Run `forkfs` to get the most up-to-date command information. If you just want to see ForkFS in
action, use the `run` command:

```sh
$ forkfs run -- <your command>
```

Or you can start a bash shell wherein any command you execute has its file operations sandboxed:

```sh
$ forkfs run bash
```

## Areas for improvement (a.k.a. limitations)

* Lazily copy files intercepted through open calls. Currently, if the file is opened for writes, we
  copy the file even if nothing is ever written to it. Additional optimizations can make this
  feature far more advanced:
    * Lazily copy changes in the background, i.e. return from the open syscall while copying in
      parallel such that the write call does not need extra I/O.
    * Use better heuristics to determine if copies are needed. For example, no copying is needed if
      the file is opened in `O_WRONLY|O_CREAT` mode and already exists. Other heuristics could be
      determined based on real-world program analysis.
    * Seeks could be kept track of such that only diffs need to be stored in the sandbox.
* Optimize operations on directories. Currently, the entire directory is (shallowly) copied if you
  run `ls` for example. We should be able to merge results from the real and sandboxed directories.
* Use tokio_uring.
* Implement more syscalls!
