//! Composition must preserve each store's retry/integrity classification.
use polaris_kernel::{
    KernelError,
    storage::{eventlog::EventLogError, registry::RegistryError},
};
use polaris_sdk::error::{ErrorKind, HasKind};

#[test]
fn storage_error_kinds_are_preserved() {
    assert_eq!(
        KernelError::from(EventLogError::Locked).kind(),
        ErrorKind::Transient,
        "a live database owner may release its lock"
    );
    assert_eq!(
        KernelError::from(EventLogError::Codec("synthetic corruption".to_owned())).kind(),
        ErrorKind::Fatal,
        "corrupt records are not retried as busy"
    );
    assert_eq!(
        KernelError::from(RegistryError::Busy).kind(),
        ErrorKind::Transient,
        "busy registry stays retryable"
    );
    assert_eq!(
        KernelError::from(RegistryError::Corrupt("synthetic corruption".to_owned())).kind(),
        ErrorKind::Fatal,
        "corrupt registry stops the component"
    );
    assert_eq!(
        KernelError::from(RegistryError::BadValue(
            "synthetic invalid input".to_owned()
        ))
        .kind(),
        ErrorKind::Permanent,
        "invalid metadata is permanent"
    );
}
