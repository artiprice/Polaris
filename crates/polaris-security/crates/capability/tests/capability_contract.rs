//! Descriptor: specs/descriptors/capability_authorization.json. Synthetic keys and mutations.
use polaris_sdk::{
    contracts::Timestamp,
    ids::{Cid, FrameId},
};
use polaris_security_capability::{
    Authority, Authorization, CapabilityError, Clock, Facts, MAX_TOKEN_BLOCKS, MAX_TOKEN_BYTES,
    ResourcePath, ResourceScope, Right, Rights, Scope, Token, attenuate, authorize,
    authorize_with_revocations,
};
use proptest::prelude::*;
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error>>;
#[derive(Debug, Clone, Copy)]
struct FakeClock(i64);
impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_second(self.0).unwrap()
    }
}
fn scope() -> Result<Scope, CapabilityError> {
    Ok(Scope {
        resource: ResourceScope::Prefix(ResourcePath::new("src")?),
        max_churn_lines: 150,
        ttl: Duration::from_secs(60),
        allow_unsafe: false,
        diff_blake3: Some(Cid::from_bytes([1; 32])),
        rights: Rights::new(&[Right::Read, Right::ProposeDiff]),
    })
}
fn facts(path: &ResourcePath) -> Facts<'_> {
    Facts {
        resource: path,
        symbol: None,
        churn_lines: 10,
        touches_unsafe: false,
        diff_blake3: Some(Cid::from_bytes([1; 32])),
        right: Right::ProposeDiff,
    }
}
fn issue(scope: &Scope) -> Result<(Authority, Token), CapabilityError> {
    let authority = Authority::new_session()?;
    let token = authority.issue_root(FrameId::from_u128(7), scope, &FakeClock(1000))?;
    Ok((authority, token))
}
fn allowed(value: Authorization) -> bool {
    matches!(value, Authorization::Allowed)
}

#[test]
fn root_token_verifies_with_public_key() -> TestResult {
    let (authority, token) = issue(&scope()?)?;
    let token = Token::from_bytes(token.to_bytes().as_bytes(), &authority.public_key())?;
    let path = ResourcePath::new("src/a.rs")?;
    assert!(allowed(authorize(
        &token,
        &facts(&path),
        &authority.public_key(),
        &FakeClock(1000)
    )?));
    assert_eq!(token.block_count(), 1);
    Ok(())
}
#[test]
fn root_token_wrong_key_rejected() -> TestResult {
    let (_, token) = issue(&scope()?)?;
    let wrong = Authority::new_session()?.public_key();
    assert!(matches!(
        Token::from_bytes(token.to_bytes().as_bytes(), &wrong),
        Err(CapabilityError::BadSignature)
    ));
    let path = ResourcePath::new("src/a.rs")?;
    assert!(matches!(
        authorize(&token, &facts(&path), &wrong, &FakeClock(1000)),
        Err(CapabilityError::BadSignature)
    ));
    Ok(())
}
#[test]
fn authority_and_token_debug_are_redacted() -> TestResult {
    let (authority, token) = issue(&scope()?)?;
    assert_eq!(format!("{authority:?}"), "Authority(<redacted>)");
    assert!(format!("{token:?}").starts_with("Token(<redacted,"));
    Ok(())
}
#[test]
fn root_scope_grants_listed_rights_only() -> TestResult {
    let mut root = scope()?;
    root.rights = Rights::new(&[Right::Read]);
    let (authority, token) = issue(&root)?;
    let path = ResourcePath::new("src/a.rs")?;
    for right in Right::ALL {
        let input = Facts {
            right,
            ..facts(&path)
        };
        assert_eq!(
            allowed(authorize(
                &token,
                &input,
                &authority.public_key(),
                &FakeClock(1000)
            )?),
            right == Right::Read
        );
    }
    Ok(())
}
#[test]
fn resource_scope_is_component_bounded() -> TestResult {
    for resource in [
        ResourceScope::Prefix(ResourcePath::new("src")?),
        ResourceScope::Exact(ResourcePath::new("src/a.rs")?),
    ] {
        let root = Scope {
            resource,
            ..scope()?
        };
        let (authority, token) = issue(&root)?;
        for (path, expect) in [
            ("src/a.rs", true),
            ("src2/a.rs", false),
            (
                "src/a.rs.bak",
                matches!(root.resource, ResourceScope::Prefix(_)),
            ),
        ] {
            let path = ResourcePath::new(path)?;
            assert_eq!(
                allowed(authorize(
                    &token,
                    &facts(&path),
                    &authority.public_key(),
                    &FakeClock(1000)
                )?),
                expect
            );
        }
    }
    Ok(())
}
#[test]
fn symbol_scope_requires_exact_path_and_symbol() -> TestResult {
    let root = Scope {
        resource: ResourceScope::Symbol {
            path: ResourcePath::new("src/a.rs")?,
            symbol: "A::run".to_owned(),
        },
        ..scope()?
    };
    let (authority, token) = issue(&root)?;
    for (path, symbol, expected) in [
        ("src/a.rs", Some("A::run"), true),
        ("src/b.rs", Some("A::run"), false),
        ("src/a.rs", Some("B::run"), false),
        ("src/a.rs", None, false),
    ] {
        let path = ResourcePath::new(path)?;
        assert_eq!(
            allowed(authorize(
                &token,
                &Facts {
                    symbol,
                    ..facts(&path)
                },
                &authority.public_key(),
                &FakeClock(1000)
            )?),
            expected
        );
    }
    Ok(())
}
#[test]
fn resource_paths_reject_aliases_and_escapes() -> TestResult {
    for path in [
        "",
        "../x",
        "src/../x",
        "./src/a",
        "/src/a",
        "C:/x",
        "C:x",
        "\\\\host\\share",
        "src//a",
        "src/a/",
        "src/a:stream",
        "src/CON",
        "src/NUL.txt",
        "src/CON .txt",
        "src/com¹.log",
        "src/a.",
        "src/a ",
        "src/%2e%2e/x",
        "src/a\0.rs",
    ] {
        assert!(ResourcePath::new(path).is_err(), "{path:?}");
    }
    assert_eq!(
        ResourcePath::new("src\\nested\\a.rs")?,
        ResourcePath::new("src/nested/a.rs")?
    );
    assert_eq!(
        ResourcePath::new("SRC/a.rs")? == ResourcePath::new("src/a.rs")?,
        cfg!(windows)
    );
    Ok(())
}
#[test]
fn expiry_uses_injected_clock_and_is_exclusive() -> TestResult {
    let (authority, token) = issue(&scope()?)?;
    let path = ResourcePath::new("src/a.rs")?;
    for (now, expected) in [
        (999, false),
        (1000, true),
        (1059, true),
        (1060, false),
        (2000, false),
    ] {
        assert_eq!(
            allowed(authorize(
                &token,
                &facts(&path),
                &authority.public_key(),
                &FakeClock(now)
            )?),
            expected
        );
    }
    Ok(())
}
#[test]
fn scope_limits_and_diff_binding_are_enforced() -> TestResult {
    let (authority, token) = issue(&scope()?)?;
    let path = ResourcePath::new("src/a.rs")?;
    for input in [
        Facts {
            churn_lines: 151,
            ..facts(&path)
        },
        Facts {
            touches_unsafe: true,
            ..facts(&path)
        },
        Facts {
            diff_blake3: None,
            ..facts(&path)
        },
        Facts {
            diff_blake3: Some(Cid::from_bytes([2; 32])),
            ..facts(&path)
        },
    ] {
        assert!(!allowed(authorize(
            &token,
            &input,
            &authority.public_key(),
            &FakeClock(1000)
        )?));
    }
    assert!(allowed(authorize(
        &token,
        &Facts {
            churn_lines: 150,
            ..facts(&path)
        },
        &authority.public_key(),
        &FakeClock(1000)
    )?));
    Ok(())
}
#[test]
fn widening_child_never_escapes_ancestor_checks() -> TestResult {
    let (authority, root) = issue(&scope()?)?;
    let wide = Scope {
        resource: ResourceScope::Prefix(ResourcePath::new("other")?),
        max_churn_lines: u32::MAX,
        ttl: Duration::from_secs(600),
        allow_unsafe: true,
        diff_blake3: None,
        rights: Rights::new(&Right::ALL),
    };
    let wide_path = attenuate(&root, &wide, &FakeClock(1000))?;
    let outside = ResourcePath::new("other/a.rs")?;
    assert!(!allowed(authorize(
        &wide_path,
        &facts(&outside),
        &authority.public_key(),
        &FakeClock(1000)
    )?));
    let wide = Scope {
        resource: scope()?.resource,
        ..wide
    };
    let child = attenuate(&root, &wide, &FakeClock(1000))?;
    let path = ResourcePath::new("src/a.rs")?;
    for input in [
        Facts {
            churn_lines: 151,
            ..facts(&path)
        },
        Facts {
            touches_unsafe: true,
            ..facts(&path)
        },
        Facts {
            diff_blake3: None,
            ..facts(&path)
        },
        Facts {
            right: Right::ExecuteTool,
            ..facts(&path)
        },
    ] {
        assert!(!allowed(authorize(
            &child,
            &input,
            &authority.public_key(),
            &FakeClock(1000)
        )?));
    }
    assert!(!allowed(authorize(
        &child,
        &facts(&path),
        &authority.public_key(),
        &FakeClock(1060)
    )?));
    Ok(())
}
#[test]
fn chain_of_three_attenuations_monotone() -> TestResult {
    let (authority, root) = issue(&scope()?)?;
    let mut tokens = vec![root];
    for limit in [100, 50, 20] {
        let parent = tokens.last().ok_or("missing parent")?;
        tokens.push(attenuate(
            parent,
            &Scope {
                max_churn_lines: limit,
                ..scope()?
            },
            &FakeClock(1000),
        )?);
    }
    let path = ResourcePath::new("src/a.rs")?;
    for token in &tokens {
        assert!(allowed(authorize(
            token,
            &facts(&path),
            &authority.public_key(),
            &FakeClock(1000)
        )?));
    }
    assert!(!allowed(authorize(
        tokens.last().ok_or("child")?,
        &Facts {
            churn_lines: 21,
            ..facts(&path)
        },
        &authority.public_key(),
        &FakeClock(1000)
    )?));
    Ok(())
}
#[test]
fn token_facts_cannot_spoof_ambient_facts_or_grants() -> TestResult {
    use biscuit_auth::{
        UnverifiedBiscuit,
        builder::{BlockBuilder, boolean, bytes, fact, int, string},
    };
    let (authority, root) = issue(&Scope {
        rights: Rights::new(&[Right::Read]),
        ..scope()?
    })?;
    let block = BlockBuilder::new()
        .fact(fact("resource", &[string("src/a.rs")]))?
        .fact(fact("churn", &[int(0)]))?
        .fact(fact("time", &[int(1_000_000_000)]))?
        .fact(fact("touches_unsafe", &[boolean(false)]))?
        .fact(fact("diff_blake3", &[bytes(&[1; 32])]))?
        .fact(fact("requested_right", &[string("read")]))?
        .fact(fact("granted_right", &[string("execute_tool")]))?;
    let malicious = UnverifiedBiscuit::from(root.to_bytes().as_bytes())?
        .append(block)?
        .to_vec()?;
    let token = Token::from_bytes(&malicious, &authority.public_key())?;
    let inside = ResourcePath::new("src/a.rs")?;
    let outside = ResourcePath::new("other/a.rs")?;
    let base = Facts {
        right: Right::Read,
        ..facts(&inside)
    };
    assert!(allowed(authorize(
        &token,
        &base,
        &authority.public_key(),
        &FakeClock(1000)
    )?));
    for input in [
        Facts {
            resource: &outside,
            ..base
        },
        Facts {
            churn_lines: 151,
            ..base
        },
        Facts {
            touches_unsafe: true,
            ..base
        },
        Facts {
            diff_blake3: None,
            ..base
        },
        Facts {
            right: Right::ExecuteTool,
            ..base
        },
    ] {
        assert!(!allowed(authorize(
            &token,
            &input,
            &authority.public_key(),
            &FakeClock(1000)
        )?));
    }
    assert!(!allowed(authorize(
        &token,
        &base,
        &authority.public_key(),
        &FakeClock(1060)
    )?));
    Ok(())
}
#[test]
fn revoking_an_ancestor_denies_all_descendants() -> TestResult {
    let (authority, root) = issue(&scope()?)?;
    let child = attenuate(&root, &scope()?, &FakeClock(1000))?;
    let revoked = root.revocation_identifiers();
    assert_eq!(revoked.len(), 1);
    let path = ResourcePath::new("src/a.rs")?;
    assert!(!allowed(authorize_with_revocations(
        &child,
        &facts(&path),
        &authority.public_key(),
        &FakeClock(1000),
        &revoked
    )?));
    Ok(())
}
#[test]
fn bounded_tokens_and_evaluation_fail_closed() -> TestResult {
    let (authority, mut token) = issue(&scope()?)?;
    assert!(
        Token::from_bytes(
            &vec![0; MAX_TOKEN_BYTES.saturating_add(1)],
            &authority.public_key()
        )
        .is_err()
    );
    while token.block_count() < MAX_TOKEN_BLOCKS {
        token = attenuate(&token, &scope()?, &FakeClock(1000))?;
    }
    let reparsed = Token::from_bytes(token.to_bytes().as_bytes(), &authority.public_key())?;
    assert_eq!(reparsed.block_count(), MAX_TOKEN_BLOCKS);
    let path = ResourcePath::new("src/a.rs")?;
    assert!(allowed(authorize(
        &reparsed,
        &facts(&path),
        &authority.public_key(),
        &FakeClock(1000)
    )?));
    assert!(matches!(
        attenuate(&token, &scope()?, &FakeClock(1000)),
        Err(CapabilityError::Limit)
    ));
    Ok(())
}
#[test]
fn datalog_fact_explosion_exhausts_budget_without_granting() -> TestResult {
    use biscuit_auth::{
        UnverifiedBiscuit,
        builder::{BlockBuilder, fact, int},
    };
    let (authority, root) = issue(&scope()?)?;
    let mut block = BlockBuilder::new()
        .rule("reach($a,$b) <- edge($a,$b)")?
        .rule("reach($a,$c) <- reach($a,$b), edge($b,$c)")?;
    for n in 0i64..128 {
        block = block.fact(fact("edge", &[int(n), int(n.saturating_add(1))]))?;
    }
    let raw = UnverifiedBiscuit::from(root.to_bytes().as_bytes())?
        .append(block)?
        .to_vec()?;
    let token = Token::from_bytes(&raw, &authority.public_key())?;
    let path = ResourcePath::new("src/a.rs")?;
    assert_eq!(
        authorize(
            &token,
            &facts(&path),
            &authority.public_key(),
            &FakeClock(1000)
        ),
        Err(CapabilityError::Limit)
    );
    Ok(())
}
#[test]
fn tampered_delegation_proof_is_rejected() -> TestResult {
    let (authority, root) = issue(&scope()?)?;
    let child = attenuate(&root, &scope()?, &FakeClock(1000))?;
    let mut raw = child.to_bytes().as_bytes().to_vec();
    let last = raw.last_mut().ok_or("empty token")?;
    *last ^= 1;
    assert!(matches!(
        Token::from_bytes(&raw, &authority.public_key()),
        Err(CapabilityError::BadSignature)
    ));
    Ok(())
}
#[test]
fn unknown_vocabulary_cannot_be_repaired_by_descendant_facts() -> TestResult {
    use biscuit_auth::{
        Biscuit, KeyPair,
        builder::{BlockBuilder, fact, int, string},
    };
    use polaris_security_capability::PublicKeyBytes;
    let key = KeyPair::new();
    let public = PublicKeyBytes::from_bytes(
        key.public()
            .to_bytes()
            .try_into()
            .map_err(|_| "public key length")?,
    )?;
    let root = Biscuit::builder()
        .fact(fact("polaris_capability_version", &[int(0)]))?
        .fact(fact("granted_right", &[string("read")]))?
        .build(&key)?;
    let child = root.append(
        BlockBuilder::new()
            .fact(fact("polaris_capability_version", &[int(1)]))?
            .fact(fact("granted_right", &[string("read")]))?,
    )?;
    let token = Token::from_bytes(&child.to_vec()?, &public)?;
    let path = ResourcePath::new("src/a.rs")?;
    assert!(!allowed(authorize(
        &token,
        &Facts {
            right: Right::Read,
            ..facts(&path)
        },
        &public,
        &FakeClock(1000)
    )?));
    Ok(())
}
#[test]
fn invalid_capability_inputs_are_not_component_fatal() -> TestResult {
    use biscuit_auth::{UnverifiedBiscuit,builder::BlockBuilder};
    use polaris_sdk::error::{ErrorKind,HasKind};
    let (authority,root)=issue(&scope()?)?;
    let invalid=UnverifiedBiscuit::from(root.to_bytes().as_bytes())?
        .append(BlockBuilder::new().check("check if resource($p), $p < 2")?)?.to_vec()?;
    let token=Token::from_bytes(&invalid,&authority.public_key())?;
    let path=ResourcePath::new("src/a.rs")?;
    let error=authorize(&token,&facts(&path),&authority.public_key(),&FakeClock(1000))
        .expect_err("a type-invalid descendant must never approve");
    assert_eq!(error.kind(),ErrorKind::Permanent,"untrusted token input must not stop the verifier component");
    let error=authority.issue_root(FrameId::from_u128(9),&Scope{ttl:Duration::MAX,..scope()?},&FakeClock(1000))
        .expect_err("unrepresentable requested lifetime");
    assert_eq!(error.kind(),ErrorKind::Permanent,"invalid TTL is a rejected scope, not clock infrastructure failure");
    Ok(())
}
#[test]
fn invalid_ttl_and_overflow_are_rejected() -> TestResult {
    for ttl in [Duration::ZERO, Duration::from_nanos(1), Duration::MAX] {
        assert!(issue(&Scope { ttl, ..scope()? }).is_err());
    }
    Ok(())
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn prop_attenuation_is_monotone(parent_limit in 0u32..200,
        children in prop::collection::vec((0u32..300,any::<bool>(),1u64..120),1..8), actual in 0u32..300,
        now in 999i64..1100, unsafe_change in any::<bool>()) {
        let root_scope=Scope{max_churn_lines:parent_limit,..scope()?};
        let (authority,root)=issue(&root_scope)?;
        let mut tokens=vec![root];
        for (child_limit,child_unsafe,ttl) in children {
            let child=attenuate(tokens.last().ok_or_else(||TestCaseError::fail("missing parent"))?,&Scope{
                max_churn_lines:child_limit,allow_unsafe:child_unsafe,ttl:Duration::from_secs(ttl),..scope()?
            },&FakeClock(1000))?;
            tokens.push(child);
        }
        let path=ResourcePath::new("src/a.rs")?;
        let input=Facts{churn_lines:actual,touches_unsafe:unsafe_change,..facts(&path)};
        let decisions=tokens.iter().map(|token|authorize(token,&input,&authority.public_key(),&FakeClock(now)).map(allowed))
            .collect::<Result<Vec<_>,_>>()?;
        for adjacent in decisions.windows(2) {
            let [parent_allowed,child_allowed]=adjacent else {return Err(TestCaseError::fail("missing adjacent states"));};
            prop_assert!(!child_allowed || *parent_allowed);
        }
    }
    #[test]
    fn prop_arbitrary_token_bytes_never_authorize(bytes in prop::collection::vec(any::<u8>(),0..2048)) {
        let authority=Authority::new_session()?;
        prop_assert!(Token::from_bytes(&bytes,&authority.public_key()).is_err());
    }
}
