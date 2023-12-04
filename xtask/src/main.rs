#![feature(lazy_cell)]

mod build;
mod cmd_util;
mod qemu;
mod tool;
mod variables;

use build::BuildArgs;
use clap::{Parser, Subcommand};
use const_format::formatcp;
use qemu::QemuArgs;
use tool::{AsmArgs, FatProbeArgs};
use variables::TARGET_ARCH;

const KERNEL_ELF_PATH: &str = formatcp!("target/{TARGET_ARCH}/kernel");
const KERNEL_BIN_PATH: &str = formatcp!("{KERNEL_ELF_PATH}.bin");

#[derive(Parser)]
#[clap(version, about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Build(BuildArgs),
    Asm(AsmArgs),
    Clean,
    Lint,
    Qemu(QemuArgs),
    FatProbe(FatProbeArgs),
    ElfExtractor,
    Env,
}

fn main() {
    #[allow(clippy::enum_glob_use)]
    use Commands::*;
    match Cli::parse().command {
        Build(args) => args.build(),
        Asm(args) => args.dump(),
        Clean => tool::clean(),
        Lint => tool::lint(),
        Qemu(args) => args.run(),
        FatProbe(args) => args.probe(),
        ElfExtractor => tool::elf_extract(),
        Env => tool::prepare_env(),
    }
}
