//! Smoke test the public facade edges without requiring credentials or host setup.

#[test]
fn every_security_child_is_reexported() {
    let _ = std::any::type_name::<polaris_security::automata::DfaScanner>();
    let _ = std::any::type_name::<polaris_security::capability::Authority>();
    let _ = std::any::type_name::<polaris_security::pii::PiiVault>();
}

use polaris_security::{rules as _, secrets as _};
