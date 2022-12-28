use std::{
    ffi::OsString,
    io,
    io::Write,
    process::{ExitCode, Termination},
};

use clap::{ArgAction, Parser};
use forkfs::forkfs;

/// A divergent history file system emulator
///
/// Under the hood, `ForkFS` creates an `OverlayFS` per session. `ForkFS` must
/// therefore be run as sudo to create these new mount points.
///
/// Note: we make no security claims. Do NOT use this tool with potentially
/// malicious software.
///
/// PS: you might also be interested in Firejail: <https://firejail.wordpress.com/>.
#[derive(Parser, Debug)]
#[clap(version, author = "Alex Saveau (@SUPERCILEX)")]
#[clap(infer_subcommands = true, infer_long_args = true)]
#[clap(next_display_order = None)]
#[clap(max_term_width = 100)]
#[command(disable_help_flag = true)]
#[command(arg_required_else_help = true)]
#[cfg_attr(test, clap(help_expected = true))]
struct ForkFs {
    /// The fork/sandbox to use
    ///
    /// Each session has its own separate view of the file system that is
    /// persistent. That is, individual command invocations build upon each
    /// other.
    ///
    /// To delete a session, unmount and delete the directory of the session in
    /// ForkFS' cache directory. For example:
    /// `sudo umount /root/.cache/forkfs/default/merged &&
    ///  sudo rm -r /root/.cache/forkfs/default` where 'default' is the session
    /// name.
    ///
    /// Note: weird things may happen if the real file system changes after
    /// establishing a session. You may want to delete all sessions to
    /// restore clean behavior in such cases.
    #[arg(short, long, short_alias = 'n', alias = "name")]
    #[arg(default_value = "default")]
    session: String,
    /// The command to run in isolation
    #[arg(required = true)]
    command: Vec<OsString>,
    #[arg(short, long, short_alias = '?', global = true)]
    #[arg(action = ArgAction::Help, help = "Print help information (use `--help` for more detail)")]
    #[arg(long_help = "Print help information (use `-h` for a summary)")]
    help: Option<bool>,
}

fn main() -> ExitCode {
    let args = ForkFs::parse();

    match forkfs(&args.session, args.command.as_slice()) {
        Ok(o) => o.report(),
        Err(err) => {
            drop(writeln!(io::stderr(), "Error: {err:?}"));
            err.report()
        }
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
