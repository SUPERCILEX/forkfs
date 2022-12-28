use std::{
    ffi::OsString,
    io,
    io::Write,
    process::{ExitCode, Termination},
};

use clap::{ArgAction, Args, Parser, Subcommand};
use error_stack::Result;
use forkfs::SessionOperand;

/// A sandboxing file system emulator
///
/// Under the hood, `ForkFS` creates an `OverlayFS` per session. `ForkFS` must
/// therefore be run as sudo to create these new mount points.
///
/// Note: we make no security claims. Do NOT use this tool with potentially
/// malicious software.
///
/// PS: you might also be interested in Firejail: <https://firejail.wordpress.com/>.
#[derive(Parser, Debug)]
#[command(version, author = "Alex Saveau (@SUPERCILEX)")]
#[command(infer_subcommands = true, infer_long_args = true)]
#[command(next_display_order = None)]
#[command(max_term_width = 100)]
#[command(disable_help_flag = true)]
#[cfg_attr(test, command(help_expected = true))]
struct ForkFs {
    #[command(subcommand)]
    cmd: Cmd,

    #[arg(short, long, short_alias = '?', global = true)]
    #[arg(action = ArgAction::Help, help = "Print help information (use `--help` for more detail)")]
    #[arg(long_help = "Print help information (use `-h` for a summary)")]
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
    /// The fork/sandbox to use
    ///
    /// If it does not exist or is inactive, it will be created and activated.
    #[arg(short = 's', long = "session", short_alias = 'n', aliases = & ["name", "id"])]
    #[arg(default_value = "default")]
    session: String,

    /// The command to run in isolation
    #[arg(required = true)]
    command: Vec<OsString>,
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
    /// Operate on all sessions
    #[arg(short = 'a', long = "all", group = "names")]
    all: bool,

    /// The session(s) to operate on
    #[arg(required = true, group = "names")]
    sessions: Vec<String>,
}

fn main() -> ExitCode {
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

fn run(Run { session, command }: Run) -> Result<(), forkfs::Error> {
    forkfs::run(&session, command.as_slice())
}

fn sessions(sessions: Sessions) -> Result<(), forkfs::Error> {
    match sessions {
        Sessions::List => forkfs::list_sessions(),
        Sessions::Stop(SessionCmd { all, sessions }) => forkfs::stop_sessions(if all {
            SessionOperand::All
        } else {
            SessionOperand::List(sessions.as_slice())
        }),
        Sessions::Delete(SessionCmd { all, sessions }) => forkfs::delete_sessions(if all {
            SessionOperand::All
        } else {
            SessionOperand::List(sessions.as_slice())
        }),
    }
}

#[cfg(test)]
mod cli_tests {
    use std::fmt::Write;

    use clap::{Command, CommandFactory};
    use expect_test::expect_file;

    use super::*;

    #[test]
    fn verify_app() {
        ForkFs::command().debug_assert();
    }

    #[test]
    #[cfg_attr(miri, ignore)] // wrap_help breaks miri
    fn help_for_review() {
        let mut command = ForkFs::command();

        command.build();

        let mut long = String::new();
        let mut short = String::new();

        write_help(&mut long, &mut command, LongOrShortHelp::Long);
        write_help(&mut short, &mut command, LongOrShortHelp::Short);

        expect_file!["../command-reference.golden"].assert_eq(&long);
        expect_file!["../command-reference-short.golden"].assert_eq(&short);
    }

    #[derive(Copy, Clone)]
    enum LongOrShortHelp {
        Long,
        Short,
    }

    fn write_help(buffer: &mut impl Write, cmd: &mut Command, long_or_short_help: LongOrShortHelp) {
        write!(
            buffer,
            "{}",
            match long_or_short_help {
                LongOrShortHelp::Long => cmd.render_long_help(),
                LongOrShortHelp::Short => cmd.render_help(),
            }
        )
        .unwrap();

        for sub in cmd.get_subcommands_mut() {
            writeln!(buffer).unwrap();
            writeln!(buffer, "---").unwrap();
            writeln!(buffer).unwrap();

            write_help(buffer, sub, long_or_short_help);
        }
    }
}
