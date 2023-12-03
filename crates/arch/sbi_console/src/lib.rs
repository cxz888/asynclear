//! SBI console driver, for text output

#![no_std]
#![feature(format_args_nl)]

use spin::Mutex;

use core::fmt::{Arguments, Result, Write};

/// 绕过所有锁打印一个字符
#[inline]
fn putchar_raw(c: usize) {
    #[allow(deprecated)]
    sbi_rt::legacy::console_putchar(c as _);
}

/// 标准输出
struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> Result {
        for c in s.chars() {
            putchar_raw(c as usize);
        }
        Ok(())
    }
}

static STDOUT: Mutex<Stdout> = Mutex::new(Stdout);

/// 输出到 stdout
#[inline]
fn stdout_puts(fmt: Arguments) {
    STDOUT.lock().write_fmt(fmt).unwrap();
}

#[inline]
pub fn print(args: Arguments) {
    stdout_puts(args);
}

/// 打印格式字符串，无换行
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::print(core::format_args!($($arg)*));
    }
}

/// 打印格式字符串，有换行
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => {
        $crate::print(core::format_args_nl!($($arg)*));
    }
}