#![feature(lazy_cell)]

mod build;
mod cmd_util;
mod ktest;
mod profiling;
mod qemu;
mod tool;
mod variables;

use build::BuildArgs;
use clap::{Parser, Subcommand};
use const_format::formatcp;
use ktest::KtestArgs;
use profiling::ProfilingArgs;
use qemu::QemuArgs;
use tool::{AsmArgs, FatProbeArgs};
use variables::TARGET_ARCH;

const KERNEL_ELF_PATH: &str = formatcp!("target/{TARGET_ARCH}/kernel");
const KERNEL_BIN_PATH: &str = formatcp!("kernel-qemu");

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
    Ktest(KtestArgs),
    /// 清除内核和用户程序的构建产物
    Clean,
    /// 对项目进行代码检查
    Lint,
    Qemu(QemuArgs),
    FatProbe(FatProbeArgs),
    /// 准备项目的开发环境，运行一次即可
    Env,
    Profiling(ProfilingArgs),
}

fn main() {
    #[allow(clippy::enum_glob_use)]
    use Commands::*;
    match Cli::parse().command {
        Build(args) => args.build(),
        Asm(args) => args.dump(),
        Ktest(args) => args.run_test(),
        Clean => tool::clean(),
        Lint => tool::lint(),
        Qemu(args) => args.run(),
        FatProbe(args) => args.probe(),
        Env => tool::prepare_env(),
        Profiling(args) => args.analyze(),
    }
}
