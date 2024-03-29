use std::{
    ffi::OsString,
    io,
    io::Write,
    process::{ExitCode, Termination},
};

use clap::{ArgAction, Args, Parser, Subcommand};
use error_stack::Result;
use forkfs::SessionOperand;

#[allow(clippy::doc_markdown)]
/// A sandboxing file system emulator
///
/// You can think of ForkFS as a lightweight container: programs still have
/// access to your real system (and can therefore jump out of the sandbox), but
/// their disk changes are re-routed to special directories without changing the
/// real file system. Under the hood, ForkFS is implemented as a wrapper around
/// OverlayFS.
///
/// Warning: we make no security claims. Do NOT use this tool with potentially
/// malicious software.
///
/// PS: you might also be interested in Firejail: <https://firejail.wordpress.com/>.
#[derive(Parser, Debug)]
#[command(version, author = "Alex Saveau (@SUPERCILEX)")]
#[command(infer_subcommands = true, infer_long_args = true)]
#[command(disable_help_flag = true)]
#[command(max_term_width = 100)]
#[cfg_attr(test, command(help_expected = true))]
struct ForkFs {
    #[command(subcommand)]
    cmd: Cmd,

    #[arg(short, long, short_alias = '?', global = true)]
    #[arg(action = ArgAction::Help, help = "Print help (use `--help` for more detail)")]
    #[arg(long_help = "Print help (use `-h` for a summary)")]
    help: Option<bool>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run commands inside the sandbox
    #[command(alias = "execute")]
    Run(Run),

    /// Manage sessions
    ///
    /// Each session has its own separate view of the file system that is
    /// persistent. That is, individual command invocations build upon each
    /// other.
    ///
    /// Actives sessions are those that are mounted, while inactive sessions
    /// remember the changes that were made within them, but are not ready to be
    /// used.
    ///
    /// Note: weird things may happen if the real file system changes after
    /// establishing a session. You may want to delete all sessions to
    /// restore clean behavior in such cases.
    #[command(subcommand)]
    Sessions(Sessions),
}

#[derive(Args, Debug)]
#[command(arg_required_else_help = true)]
struct Run {
    /// The command to run in isolation
    #[arg(required = true)]
    command: Vec<OsString>,

    /// The fork/sandbox to use
    ///
    /// If it does not exist or is inactive, it will be created and activated.
    #[arg(short = 's', long = "session", short_alias = 'n', aliases = & ["name", "id"])]
    #[arg(default_value = "default")]
    session: String,
}

#[derive(Subcommand, Debug)]
enum Sessions {
    /// List sessions
    ///
    /// `[active]` sessions are denoted with brackets while `inactive` sessions
    /// are bare.
    #[command(alias = "ls")]
    List,

    /// Unmount active sessions
    #[command(alias = "close")]
    Stop(SessionCmd),

    /// Delete sessions
    #[command(alias = "destroy")]
    Delete(SessionCmd),
}

#[derive(Args, Debug)]
#[command(arg_required_else_help = true)]
struct SessionCmd {
    /// The session(s) to operate on
    #[arg(required = true, group = "names")]
    sessions: Vec<String>,

    /// Operate on all sessions
    #[arg(short = 'a', long = "all", group = "names")]
    all: bool,
}

fn main() -> ExitCode {
    #[cfg(not(debug_assertions))]
    error_stack::Report::install_debug_hook::<std::panic::Location>(|_, _| {});

    let args = ForkFs::parse();

    match forkfs(args) {
        Ok(o) => o.report(),
        Err(err) => {
            drop(writeln!(io::stderr(), "Error: {err:?}"));
            err.report()
        }
    }
}

fn forkfs(ForkFs { cmd, help: _ }: ForkFs) -> Result<(), forkfs::Error> {
    match cmd {
        Cmd::Run(r) => run(r),
        Cmd::Sessions(s) => sessions(s),
    }
}

fn run(Run { command, session }: Run) -> Result<(), forkfs::Error> {
    forkfs::run(&session, command.as_slice())
}

fn sessions(sessions: Sessions) -> Result<(), forkfs::Error> {
    match sessions {
        Sessions::List => forkfs::list_sessions(),
        Sessions::Stop(SessionCmd { sessions, all }) => forkfs::stop_sessions(if all {
            SessionOperand::All
        } else {
            SessionOperand::List(sessions.as_slice())
        }),
        Sessions::Delete(SessionCmd { sessions, all }) => forkfs::delete_sessions(if all {
            SessionOperand::All
        } else {
            SessionOperand::List(sessions.as_slice())
        }),
    }
}

#[cfg(test)]
mod cli_tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn verify_app() {
        ForkFs::command().debug_assert();
    }

    #[test]
    fn help_for_review() {
        supercilex_tests::help_for_review(ForkFs::command());
    }
}
