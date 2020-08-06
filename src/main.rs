#![warn(trivial_numeric_casts)]
//! bfc is a highly optimising compiler for BF.

#[macro_use]
extern crate matches;

use diagnostics::Info;
use getopts::Options;
use std::env;
use std::path::Path;

mod bfir;
mod bounds;
mod diagnostics;
mod execution;
mod llvm;
mod peephole;
mod shell;
mod io;

#[cfg(test)]
mod llvm_tests;
#[cfg(test)]
mod peephole_tests;
#[cfg(test)]
mod soundness_tests;

/// Convert "foo.bf" to "foo".
fn executable_name(bf_path: &str) -> String {
    let bf_file_name = Path::new(bf_path).file_name().unwrap().to_str().unwrap();

    let mut name_parts: Vec<_> = bf_file_name.split('.').collect();
    let parts_len = name_parts.len();
    if parts_len > 1 {
        name_parts.pop();
    }

    name_parts.join(".")
}

fn print_usage(bin_name: &str, opts: Options) {
    let brief = format!("Usage: {} SOURCE_FILE [options]", bin_name);
    print!("{}", opts.usage(&brief));
}

fn link_object_file(
    object_file_path: &str,
    executable_path: &str,
    target_triple: Option<String>,
) -> Result<(), Info> {
    // Link the object file.
    let clang_args = if let Some(ref target_triple) = target_triple {
        vec![
            object_file_path,
            "-target",
            &target_triple,
            "-o",
            &executable_path[..],
        ]
    } else {
        vec![object_file_path, "-o", &executable_path[..]]
    };

    shell::run_shell_command("clang", &clang_args[..])
}

fn strip_executable(executable_path: &str) -> Result<(), Info> {
    let strip_args = ["-s", &executable_path[..]];
    shell::run_shell_command("strip", &strip_args[..])
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<_> = env::args().collect();

    let mut opts = Options::new();

    opts.optflag("h", "help", "print usage");
    opts.optflag("v", "version", "print bfc version");
    opts.optflag("", "dump-llvm", "print LLVM IR generated");
    opts.optflag("", "dump-ir", "print BF IR generated");

    opts.optopt("O", "opt", "optimization level (0 to 2)", "LEVEL");
    opts.optopt("", "llvm-opt", "LLVM optimization level (0 to 3)", "LEVEL");
    opts.optopt(
        "",
        "passes",
        "limit bfc optimisations to those specified",
        "PASS-SPECIFICATION",
    );
    opts.optopt(
        "",
        "strip",
        "strip symbols from the binary (default: yes)",
        "yes|no",
    );

    let default_triple_cstring = llvm::get_default_target_triple();
    let default_triple = default_triple_cstring.to_str().unwrap();

    opts.optopt(
        "",
        "target",
        &format!("LLVM target triple (default: {})", default_triple),
        "TARGET",
    );

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(_) => {
            print_usage(&args[0], opts);
            std::process::exit(1);
        }
    };

    if matches.opt_present("h") {
        print_usage(&args[0], opts);
        return;
    }

    if matches.opt_present("v") {
        println!("bfc {}", VERSION);
        return;
    }

    if matches.free.len() != 1 {
        print_usage(&args[0], opts);
        std::process::exit(1);
    }

    match io::compile_file(&matches) {
        Ok(_) => {}
        Err(errors) => {
            for error in errors {
                eprintln!("{}", error);
            }
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn executable_name_bf() {
        assert_eq!(executable_name("foo.bf"), "foo");
    }

    #[test]
    fn executable_name_b() {
        assert_eq!(executable_name("foo_bar.b"), "foo_bar");
    }

    #[test]
    fn executable_name_relative_path() {
        assert_eq!(executable_name("bar/baz.bf"), "baz");
    }
}
