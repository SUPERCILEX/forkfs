use std::{
    ffi::CString,
    os::unix::prelude::OsStrExt,
    path::PathBuf,
};

use libc::user_regs_struct;
use nix::{
    libc,
    NixPath,
    sys::{
        ptrace::{AddressType, getregs, read, setregs, syscall, traceme, write},
        wait::{waitpid, WaitStatus::Exited},
    },
    unistd::{
        execvp, fork,
        ForkResult::{Child, Parent},
        Pid,
    },
};

use crate::{
    CliResult,
    divergence::FileChanges,
    errors::{CliExitError, CliExitNixWrapper},
    interceptor::ExitSyscallOp::MutatingOpen,
};

const MUTATING_OPEN_FLAGS: [i32; 4] = [libc::O_CREAT, libc::O_TRUNC, libc::O_WRONLY, libc::O_RDWR];
const STACK_RED_ZONE: u64 = 128;

enum ExitSyscallOp {
    MutatingOpen(PathBuf),
}

pub fn run_intercepted_program(program: Vec<String>, session: PathBuf) -> CliResult<()> {
    let pid = unsafe { fork() }.unwrap();
    match pid {
        Child => exec_program(program),
        Parent { child } => {
            let mut changes = FileChanges::new(session.with_extension("changes"), session);
            changes.restore_from_disk()?;
            intercept_syscalls(child, changes)
        }
    }
}

fn exec_program(args: Vec<String>) -> CliResult<()> {
    traceme().unwrap();

    let arg_cstrs: Vec<CString> = args
        .iter()
        .map(|arg| CString::new(arg.as_bytes()).unwrap())
        .collect();

    execvp(&arg_cstrs[0], &arg_cstrs)
        .map(|_| ())
        .with_backing_code(|| {
            let command_reconstruction = arg_cstrs.iter().fold(
                String::with_capacity(
                    arg_cstrs.len() + arg_cstrs.iter().map(|s| s.len()).sum::<usize>(),
                ),
                |acc, str| acc + str.to_str().unwrap() + " ",
            );

            format!(
                "Failed to execute '{}'",
                &command_reconstruction[..command_reconstruction.len() - 1]
            )
        })
}

fn intercept_syscalls(child: Pid, mut changes: FileChanges) -> CliResult<()> {
    let mut exit_op: Option<ExitSyscallOp> = None;

    loop {
        if let Some(code) = wait_for_exit(child) {
            if code != exitcode::OK {
                break Err(CliExitError { code, source: None });
            }
            break Ok(());
        }

        match exit_op {
            Some(_) => {
                exit_op = None;
            }
            None => {
                let regs = getregs(child).unwrap();
                match regs.orig_rax as i64 {
                    libc::SYS_openat => handle_enter_open(child, &mut changes, regs, &mut exit_op)?,
                    _ => {}
                }

                // TODO handle open, unlink*, mkdir*, rename*, rmdir*, creat*, link*, symlink*,
                //  chmod*, chown*, lchown*, utime*, mknod*, *xattr*, utimes*, inotify_add_watch*,
                //  futimesat, mmap*
            }
        }

        syscall(child, None).unwrap();
    }
}

fn handle_enter_open(
    pid: Pid,
    changes: &mut FileChanges,
    mut regs: user_regs_struct,
    exit_op: &mut Option<ExitSyscallOp>,
) -> CliResult<()> {
    if regs.rdi as i32 != libc::AT_FDCWD {
        todo!("Handle params other than AT_FDCWD");
        // Maybe use readlink on /proc/$X/fd/$Y
    }

    let path = PathBuf::from(read_string_mem(pid, regs.rsi));
    let existing_change = changes.includes(&path);
    let can_modify = (regs.rdx as i32).has_any_flags(&MUTATING_OPEN_FLAGS);
    if existing_change || can_modify {
        let relocated = if existing_change {
            changes.destination(&path)
        } else {
            changes.on_file_modified(&path)?
        };

        let mut nul_relocated = relocated.as_os_str().as_bytes().to_vec();
        nul_relocated.push(0);
        let new_filename_address = regs.rsp - STACK_RED_ZONE - nul_relocated.len() as u64;
        write_mem(pid, new_filename_address, &nul_relocated);

        regs.rsi = new_filename_address;
        setregs(pid, regs).unwrap();

        *exit_op = Some(MutatingOpen(relocated));
    }

    Ok(())
}

fn wait_for_exit(pid: Pid) -> Option<i32> {
    let status = waitpid(pid, None).unwrap();
    if let Exited(interrupt_pid, exitcode) = status {
        if pid == interrupt_pid {
            return Some(exitcode);
        }
    }
    None
}

fn read_string_mem(pid: Pid, mut ptr: u64) -> String {
    let mut chars = Vec::new();
    loop {
        let word = read(pid, ptr as AddressType).unwrap() as u64;
        for i in 0..8 {
            let c = (word >> (i * 8)) & 0xFF;
            if c == 0 {
                return String::from_utf8(chars).unwrap();
            }

            chars.push(c as u8);
        }
        ptr += 8;
    }
}

fn write_mem(pid: Pid, mut ptr: u64, bytes: &[u8]) {
    let mut i = 0;

    while i <= bytes.len() - 8 {
        unsafe {
            let word = *(bytes.as_ptr().add(i) as *const u64);
            write(pid, ptr as AddressType, word as AddressType).unwrap();
        }

        i += 8;
        ptr += 8;
    }

    if i != bytes.len() {
        let mut word = 0;
        let mut j = 0;

        for byte in &bytes[i..] {
            word |= (*byte as u64) << (j * 8);
            j += 1;
        }

        let existing_mem = read(pid, ptr as AddressType).unwrap() as u64;
        word |= existing_mem & (!0u64 << (j * 8));
        unsafe {
            write(pid, ptr as AddressType, word as AddressType).unwrap();
        }
    }
}

trait FlagUtils {
    fn has_any_flags(self, flags: &[i32]) -> bool;
}

impl FlagUtils for i32 {
    fn has_any_flags(self, flags: &[i32]) -> bool {
        return flags.iter().any(|flag| (self & *flag) == *flag);
    }
}
