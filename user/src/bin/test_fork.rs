#![no_std]
#![no_main]

#[macro_use]
extern crate user;

use user::{exit, fork, test_main, wait};

const MAX_CHILD: usize = 8;

#[no_mangle]
pub fn main() -> i32 {
    test_main("test_fork", || {
        for i in 0..MAX_CHILD {
            let pid = fork();
            if pid == 0 {
                println!("I am child {}", i);
                exit(0);
            } else {
                println!("forked child pid = {}", pid);
            }
            assert!(pid > 0);
        }
        let mut exit_code: i32 = 0;
        for _ in 0..MAX_CHILD {
            if wait(&mut exit_code) <= 0 {
                panic!("wait stopped early");
            }
        }
        if wait(&mut exit_code) > 0 {
            panic!("wait got too many");
        }
    });
    0
}
