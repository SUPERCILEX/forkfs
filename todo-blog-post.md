---
title: "ForkFS: A divergent history file system"
subtitle: Isolate a process's changes to your files

categories: [Future, Tools, File-System]
redirect_from:
- /blog/forkfs/
---

* TOC
  {:toc}

ForkFS, as the name suggests, allows you to "fork" your file system for a process or processes. This
means you can run programs which modify your file system and then inspect those changes, deciding
which to keep or discard.

## That's cool, but why would I want this?

Anytime you may want to roll back the changes a program makes, ForkFS is your friend:

- Need to debug someone else's repo, but you don't want potential changes to your home directory
  (like caches) to stick around?
- Not sure if that `rm/mv/cp` command will do what you want?
- Want to limit which files a program can see without the hassle of spinning up a VM?

Give ForkFS a spin.

## How does it work?

The basic idea of ForkFS is to intercept file I/O system calls and reroute them to a safe directory
which can later be inspected.

### Intercepting system calls

ForkFS uses [ptrace(2)](https://man7.org/linux/man-pages/man2/ptrace.2.html) to intercept the system
calls of its child process. 
