use std::process::exit;

use anyhow::Result;
use clap_verbosity_flag::Verbosity;
use simple_logger::SimpleLogger;
use structopt::{clap::AppSettings, StructOpt};

use crate::{
    errors::CliResult,
    forks::{apply_fork, diff_fork, get_fork, list_forks, remove_forks},
    interceptor::run_intercepted_program,
};

mod errors;
mod forks;
mod interceptor;

/// A divergent history file system
#[derive(Debug, StructOpt)]
#[structopt(
author = "Alex Saveau (@SUPERCILEX)",
global_settings = & [AppSettings::InferSubcommands, AppSettings::ColoredHelp],
)]
struct ForkFS {
    #[structopt(flatten)]
    verbose: Verbosity,

    #[structopt(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, StructOpt)]
enum Cmd {
    /// Run a program in a forked file system
    Run(Run),
    /// Manage forked file systems
    Forks(Forks),
}

#[derive(Debug, StructOpt)]
struct Run {
    /// The FS fork to use
    #[structopt(
    long = "fork",
    alias = "session",
    short = "s", default_value = "",
    validator = validate_fork_name
    )]
    fork: String,
    /// The program to run
    #[structopt(required = true)]
    program: Vec<String>,
}

#[derive(Debug, StructOpt)]
struct Forks {
    #[structopt(subcommand)]
    cmd: Option<ForksCmd>,
}

#[derive(Debug, StructOpt)]
enum ForksCmd {
    /// List available forks
    List(ListForks),
    /// Diff a forked file system's changes with ground truth (the actual FS)
    Diff(DiffFork),
    /// Apply the changes from a fork to ground truth (the actual FS)
    Apply(ApplyFork),
    /// Delete forks
    Remove(RemoveForks),
}

#[derive(Debug, StructOpt)]
struct ListForks {}

#[derive(Debug, StructOpt)]
struct DiffFork {
    /// The FS fork to diff
    fork: String,
}

#[derive(Debug, StructOpt)]
struct ApplyFork {
    /// The FS fork to apply
    fork: String,
}

#[derive(Debug, StructOpt)]
struct RemoveForks {
    /// Remove all forks
    #[structopt(long = "all", short = "a")]
    all: bool,
    /// The FS fork(s) to remove
    #[structopt(
    conflicts_with = "all",
    required_unless = "all",
    empty_values = false,
    validator = validate_fork_name
    )]
    forks: Vec<String>,
}

fn main() {
    match wrapped_main() {
        Err(error) => {
            eprintln!("{:?}", error.wrapped);
            exit(error.code);
        }
        _ => (),
    }
}

fn wrapped_main() -> CliResult<()> {
    let args: ForkFS = ForkFS::from_args();
    SimpleLogger::new()
        .with_level(args.verbose.log_level().unwrap().to_level_filter())
        .init()
        .unwrap();

    match args.cmd {
        Cmd::Run(options) => run(options),
        Cmd::Forks(options) => forks(options),
    }
}

fn run(options: Run) -> CliResult<()> {
    let fork = get_fork(options.fork)?;

    println!(
        "Using fork '{}'",
        fork.file_name().unwrap().to_str().unwrap()
    );
    run_intercepted_program(options.program, fork)
}

fn forks(options: Forks) -> CliResult<()> {
    match options.cmd {
        None | Some(ForksCmd::List(_)) => list_forks(),
        Some(ForksCmd::Diff(options)) => diff_fork(options.fork),
        Some(ForksCmd::Apply(options)) => apply_fork(options.fork),
        Some(ForksCmd::Remove(options)) => remove_forks(options.forks),
    }
}

fn validate_fork_name(fork: String) -> Result<(), String> {
    if fork != sanitize_filename::sanitize(&fork) {
        return Err(String::from("Fork name cannot look like a file path"));
    }
    Ok(())
}
