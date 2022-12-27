use std::{
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use libc::user_regs_struct;
use log::Metadata;
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
    errors::{CliExitAnyhowWrapper, CliExitError, CliExitNixWrapper, IoResultUtils},
    interceptor::ExitSyscallOp::Ignore,
};

const MUTATING_OPEN_FLAGS: [i32; 4] = [libc::O_CREAT, libc::O_TRUNC, libc::O_WRONLY, libc::O_RDWR];
const STACK_RED_ZONE: u64 = 128;

enum ExitSyscallOp {
    Ignore,
}

pub fn run_intercepted_program(program: Vec<String>, session: PathBuf) -> CliResult<()> {
    let pid = unsafe { fork() }.unwrap();
    match pid {
        Child => exec_program(program),
        Parent { child } => {
            let mut changes = FileChanges::new(session.join("changes.log"), session);
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
                    libc::SYS_newfstatat => {
                        handle_enter_newfstatat(child, &mut changes, regs, &mut exit_op)?
                    }
                    libc::SYS_faccessat2 => {
                        handle_enter_faccessat2(child, &mut changes, regs, &mut exit_op)?
                    }
                    libc::SYS_unlinkat => {
                        handle_enter_unlink(child, &mut changes, regs, &mut exit_op)?
                    }
                    // TODO support fork
                    _ => exit_op = Some(Ignore),
                }
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
    let path = read_path_from_v2_syscall(pid, regs)?;

    let has_changed_parent = changes.is_direct_child_of_included_parent(&path);
    let is_regular_file = || {
        let result = path.metadata();
        let is_file = result.as_ref().map(|metadata| metadata.is_file()).ok() == Some(true);
        let is_changed_dir =
            result.as_ref().map(|metadata| metadata.is_dir()).ok() == Some(true);

        result.as_ref().does_not_exist() || is_file || is_changed_dir
    };
    let existing_change = changes.includes(&path);
    let can_modify = (regs.rdx as i32).has_any_flags(&MUTATING_OPEN_FLAGS);

    if (existing_change || can_modify || has_changed_parent) && is_regular_file() {
        let relocated = if existing_change {
            changes.destination(&path)
        } else if has_changed_parent {
            changes.on_read_dir(&path)?
        } else {
            changes.on_file_modified(&path)?
        };

        write_path_mem(pid, &mut regs, &relocated);

        *exit_op = Some(Ignore);
    }

    Ok(())
}

fn handle_enter_newfstatat(
    pid: Pid,
    changes: &mut FileChanges,
    mut regs: user_regs_struct,
    exit_op: &mut Option<ExitSyscallOp>,
) -> CliResult<()> {
    let path = read_path_from_v2_syscall(pid, regs)?;

    if changes.includes(&path) {
        write_path_mem(pid, &mut regs, &changes.destination(&path));
    }

    *exit_op = Some(Ignore);

    Ok(())
}

fn handle_enter_faccessat2(
    pid: Pid,
    changes: &mut FileChanges,
    mut regs: user_regs_struct,
    exit_op: &mut Option<ExitSyscallOp>,
) -> CliResult<()> {
    let path = read_path_from_v2_syscall(pid, regs)?;

    if changes.includes(&path) {
        write_path_mem(pid, &mut regs, &changes.destination(&path));
    }

    *exit_op = Some(Ignore);

    Ok(())
}

fn handle_enter_unlink(
    pid: Pid,
    changes: &mut FileChanges,
    mut regs: user_regs_struct,
    exit_op: &mut Option<ExitSyscallOp>,
) -> CliResult<()> {
    let path = read_path_from_v2_syscall(pid, regs)?;
    let relocated = if changes.includes(&path) {
        changes.destination(&path)
    } else {
        changes.on_file_removed(&path)?
    };

    write_path_mem(pid, &mut regs, &relocated);

    *exit_op = Some(Ignore);

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

fn read_path_from_v2_syscall(pid: Pid, regs: user_regs_struct) -> CliResult<PathBuf> {
    let mut path = PathBuf::from(read_string_mem(pid, regs.rsi));
    if !path.is_absolute() && regs.rdi as i32 != libc::AT_FDCWD {
        let link = format!("/proc/{}/fd/{}", pid, regs.rdi);
        path = fs::read_link(&link)
            .with_context(|| format!("Failed to read symlink {:?}", link))
            .with_code(exitcode::IOERR)?;
    }
    Ok(path)
}

fn write_path_mem(pid: Pid, regs: &mut user_regs_struct, relocated: &Path) {
    let mut nul_relocated = Vec::with_capacity(relocated.len() + 1);
    nul_relocated.extend_from_slice(relocated.to_str().unwrap().as_bytes());
    nul_relocated.push(0);

    let new_filename_address = regs.rsp - STACK_RED_ZONE - nul_relocated.len() as u64;
    write_mem(pid, new_filename_address, &nul_relocated);

    regs.rsi = new_filename_address;
    setregs(pid, *regs).unwrap();
}

trait FlagUtils {
    fn has_any_flags(self, flags: &[i32]) -> bool;
}

impl FlagUtils for i32 {
    fn has_any_flags(self, flags: &[i32]) -> bool {
        return flags.iter().any(|flag| (self & *flag) == *flag);
    }
}
