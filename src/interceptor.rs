use std::{
    ffi::CString,
    fs,
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::Context;
use libc::user_regs_struct;
use log::info;
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
use path_absolutize::Absolutize;
use radix_trie::Trie;

use crate::{
    CliResult,
    errors::{CliExitAnyhowWrapper, CliExitError, CliExitNixWrapper},
    interceptor::ExitSyscallOp::MutatingOpen,
};

const MUTATING_OPEN_FLAGS: [u64; 2] = [libc::O_CREAT as u64, libc::O_TRUNC as u64];
const STACK_RED_ZONE: u64 = 128;

enum ExitSyscallOp {
    MutatingOpen(PathBuf),
}

pub fn run_intercepted_program(program: Vec<String>, session: PathBuf) -> CliResult<()> {
    let pid = unsafe { fork() }.unwrap();
    match pid {
        Child => exec_program(program),
        Parent { child } => intercept_syscalls(child, session),
    }
}

fn exec_program(args: Vec<String>) -> CliResult<()> {
    traceme().unwrap();

    let arg_cstrs: Vec<CString> = args
        .iter()
        .map(|arg| CString::new(arg.as_bytes()).unwrap())
        .collect();

    execvp(&arg_cstrs[0].clone(), &arg_cstrs)
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

fn intercept_syscalls(child: Pid, fork_path: PathBuf) -> CliResult<()> {
    let mut redirected_fds = HashMap::new(); // TODO needed?
    let mut exit_op: Option<ExitSyscallOp> = None;

    loop {
        match wait_for_exit(child) {
            Some(code) => {
                if code != exitcode::OK {
                    return Err(CliExitError {
                        code,
                        wrapped: None,
                    });
                }
                break;
            }
            None => {}
        }

        match exit_op {
            Some(MutatingOpen(path)) => {
                handle_exit_open(child, &mut redirected_fds, path);
                exit_op = None;
            }
            None => {
                let regs = getregs(child).unwrap();
                if regs.orig_rax == libc::SYS_openat as u64 {
                    handle_enter_open(child, &fork_path, regs, &mut exit_op)?;
                }

                // TODO handle open, unlink*, mkdir*, rename*, rmdir*, creat*, link*, symlink*,
                //  chmod*, chown*, lchown*, utime*, mknod*, *xattr*, utimes*, inotify_add_watch*,
                //  futimesat, mmap*
            }
        }

        syscall(child, None).unwrap();
    }

    Ok(())
}

fn handle_enter_open(
    pid: Pid,
    fork_path: &PathBuf,
    mut regs: user_regs_struct,
    exit_op: &mut Option<ExitSyscallOp>,
) -> CliResult<()> {
    if regs.rdi as i32 != libc::AT_FDCWD {
        todo!("Handle params other than AT_FDCWD");
        // Maybe use readlink on /proc/$X/fd/$Y
    }

    let flags = regs.rdx;
    if MUTATING_OPEN_FLAGS
        .iter()
        .any(|flag| (flags & *flag) == *flag)
    {
        let path_string = mem_to_string(pid, regs.rsi);
        let path = Path::new(&path_string);
        let relocated = fork_path.join(path.absolutize().unwrap().strip_prefix("/").unwrap());
        let relocated_parent = relocated.parent().unwrap();

        info!("Rewrote path {:?} to {:?}", path_string, relocated);

        fs::create_dir_all(relocated_parent)
            .context(format!("Failed to create directory {:?}", relocated_parent))
            .with_code(exitcode::IOERR)?;
        if !relocated.exists() && path.exists() {
            info!("Copying file {:?} to {:?}", path, relocated);
            fs::copy(path, &relocated)
                .context(format!("Copy from {:?} to {:?} failed", path, relocated))
                .with_code(exitcode::IOERR)?;
        }

        let mut nul_relocated = relocated.as_os_str().as_bytes().to_vec();
        nul_relocated.push(0);
        let new_filename_address = regs.rsp - STACK_RED_ZONE - nul_relocated.len() as u64;
        bytes_to_mem(pid, new_filename_address, &nul_relocated);

        regs.rsi = new_filename_address;
        setregs(pid, regs).unwrap();

        *exit_op = Option::from(MutatingOpen(relocated));
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
    return None;
}

fn mem_to_string(pid: Pid, mut ptr: u64) -> String {
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

fn bytes_to_mem(pid: Pid, mut ptr: u64, bytes: &[u8]) {
    let mut i = 0;

    while i < bytes.len() - 8 {
        unsafe {
            let word = *((i + bytes.as_ptr() as usize) as *const u64);
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
        word |= existing_mem & (!0u64 << j * 8);
        unsafe {
            write(pid, ptr as AddressType, word as AddressType).unwrap();
        }
    }
}
