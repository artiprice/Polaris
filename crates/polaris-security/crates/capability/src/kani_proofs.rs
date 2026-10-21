//! Isolated expiry arithmetic proof; does not claim to prove Datalog or cryptography.
use crate::{CapabilityError, scope::expiry};

#[kani::proof]
fn expiry_never_wraps_or_extends_lifetime() {
    let issued: i64 = kani::any();
    let lifetime: u128 = kani::any();
    let result = expiry(issued, lifetime);
    match i64::try_from(lifetime) {
        Err(_) => assert_eq!(result, Err(CapabilityError::Clock)),
        Ok(0) => assert_eq!(result, Err(CapabilityError::InvalidScope)),
        Ok(duration) => {
            let expected = i128::from(issued).checked_add(i128::from(duration));
            assert!(expected.is_some());
            if let Some(expected) = expected {
                match i64::try_from(expected) {
                    Ok(exact) => assert_eq!(result, Ok(exact)),
                    Err(_) => assert_eq!(result, Err(CapabilityError::Clock)),
                }
            }
        }
    }
}
