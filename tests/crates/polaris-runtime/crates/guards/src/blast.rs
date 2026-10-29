// Unit tests of crates/polaris-runtime/crates/guards/src/blast.rs (`workload_tests`); compiled into that module via #[path].
use super::{Confidence, ShellKind, WorkspacePath, classify};
use std::time::Instant;

#[test]
fn shell_classifier_100k_command_workload() {
    let cwd = WorkspacePath::root();
    let target = WorkspacePath::new("target").expect("test path");
    let commands = [
        "rm -rf target",
        "Remove-Item -Path build -Recurse",
        "git reset --hard",
        "echo output > generated.txt",
        "rm -rf $TARGET",
        "del /s /q build",
    ];
    let start = Instant::now();
    let mut unknown = 0_u64;
    for index in 0..100_000_usize {
        let shell = match index % 3 {
            0 => ShellKind::Posix,
            1 => ShellKind::PowerShell,
            _ => ShellKind::Cmd,
        };
        let result = std::hint::black_box(classify(
            commands[index % commands.len()],
            &cwd,
            shell,
            Some(&target),
        ));
        if result.confidence == Confidence::Unknown {
            unknown = unknown.saturating_add(1);
        }
    }
    let elapsed = start.elapsed();
    println!(
        "shell_classifier_commands=100000;unknown={unknown};elapsed_ms={};commands_per_second={}",
        elapsed.as_millis(),
        100_000_u128.saturating_mul(1_000) / elapsed.as_millis().max(1)
    );
}
