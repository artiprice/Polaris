// Unit tests of crates/polaris-kernel/crates/supervisor/src/worker.rs (`exit_tests`); compiled into that module via #[path].
use super::worker_exit;
use crate::ExitClass;

#[test]
fn clean_worker_exit_is_success_and_signal_or_ntstatus_is_a_crash() {
    assert!(worker_exit(Some(0)).is_ok());
    for code in [None, Some(i32::from_le_bytes([5, 0, 0, 192]))] {
        let Err(error) = worker_exit(code) else {
            panic!("crash was accepted as clean");
        };
        assert_eq!(error.exit_class(), ExitClass::Crashed);
    }
    let Err(error) = worker_exit(Some(17)) else {
        panic!("failure was accepted as clean");
    };
    assert_eq!(error.exit_class(), ExitClass::Failed);
}
