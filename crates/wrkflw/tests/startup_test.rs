//! The binary starts, on every platform. The other tests that run it need a
//! POSIX shell, so without this one nothing starts it on Windows, where the
//! 1 MiB main thread stack is the first thing an oversized future runs into.

use std::process::Command;

#[test]
fn the_binary_starts() {
    for args in [&["--help"][..], &["gate", "--help"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_wrkflw"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}: {:?}", args, output);
    }
}
