//! The binary run without a terminal.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_sharks");

#[test]
fn print_sizes_itself_from_the_environment() {
    let rows = |lines: &str| {
        let out = Command::new(BIN)
            .args(["--print", "--seed", "7", "--color", "mono"])
            .env_clear()
            .env("COLUMNS", "50")
            .env("LINES", lines)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().lines().count()
    };
    assert_eq!(rows("7"), 7);
    assert_eq!(rows("0"), 24, "LINES=0 falls back to 24");
}

#[test]
fn bad_options_fail_with_a_message() {
    for args in [&["-k", ","][..], &["--clock=sundial"], &["-f", "0"]] {
        let out = Command::new(BIN).args(args).env_clear().output().unwrap();
        assert!(!out.status.success(), "{args:?} was accepted");
        assert!(
            String::from_utf8_lossy(&out.stderr).starts_with("sharks: "),
            "{args:?}"
        );
    }
}
