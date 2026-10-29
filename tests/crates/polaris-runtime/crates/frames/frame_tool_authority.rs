use polaris_runtime_frames::{
    Active, AgentContextFrame, AgentRole, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
    scope_is_subset,
};
use polaris_sdk::{
    contracts::Timestamp,
    effort::Effort,
    ids::{Cid, FrameId},
};
use polaris_security_capability::{
    Authority, Clock, ResourcePath, ResourceScope, Right, Rights, Scope, Token,
};
use std::time::Duration;

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp::UNIX_EPOCH
    }
}

fn scope(resource: ResourceScope) -> Scope {
    Scope {
        resource,
        max_churn_lines: 100,
        ttl: Duration::from_secs(3_600),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read, Right::ExecuteTool]),
    }
}

#[test]
fn workspace_scope_attenuates_without_widening() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = scope(ResourceScope::Workspace);
    let mut narrowed = scope(ResourceScope::Prefix(ResourcePath::new("src")?));
    narrowed.rights = Rights::new(&[Right::Read]);
    assert!(scope_is_subset(&workspace, &narrowed).is_ok());
    assert!(scope_is_subset(&narrowed, &workspace).is_err());

    let exact = scope(ResourceScope::Exact(ResourcePath::new("src/main.rs")?));
    assert!(scope_is_subset(&exact, &workspace).is_err());
    let symbol = scope(ResourceScope::Symbol {
        path: ResourcePath::new("src/main.rs")?,
        symbol: "main".to_owned(),
    });
    assert!(scope_is_subset(&symbol, &workspace).is_err());

    let mut wider_right = narrowed.clone();
    wider_right.rights = Rights::new(&[Right::Read, Right::ExecuteTool]);
    assert!(scope_is_subset(&narrowed, &wider_right).is_err());
    let mut wider_churn = narrowed.clone();
    wider_churn.max_churn_lines = 101;
    assert!(scope_is_subset(&narrowed, &wider_churn).is_err());
    Ok(())
}

fn active_frame(
    id: FrameId,
    scope: Scope,
) -> Result<(AgentContextFrame<Active>, Token), Box<dyn std::error::Error>> {
    let authority = Authority::new_session()?;
    let token = authority.issue_root(id, &scope, &TestClock)?;
    let frame = AgentContextFrame::new_root(
        id,
        AgentRole::Orchestrator,
        token.clone(),
        FrameScope {
            capability: scope,
            expires_at_micros: 3_600_000_000,
        },
        ResourceQuota {
            cpu_millis: 10_000,
            memory_mb: 1_024,
            network: Default::default(),
            max_steps: 8,
            max_depth: 3,
            token_budget: 1_000,
        },
        ScratchpadRef::new(Cid::from_bytes([9; 32])),
    )
    .activate(
        ModelSelection {
            model: polaris_sdk::ids::ModelId::new("test/model")?,
            maximum_effort: Effort::Low,
        },
        Effort::Low,
    )?;
    Ok((frame, token))
}

#[test]
fn binding_matches_only_actual_frame_token() -> Result<(), Box<dyn std::error::Error>> {
    let id = FrameId::new_v7();
    let resource = ResourceScope::Workspace;
    let (frame, token) = active_frame(id, scope(resource.clone()))?;
    let binding = frame.tool_binding();
    assert_eq!(binding.frame_id(), id);
    assert_eq!(binding.root_frame_id(), id);
    assert_eq!(binding.role(), AgentRole::Orchestrator);
    assert!(binding.matches_token(&token));
    assert_eq!(format!("{binding:?}"), "FrameToolBinding(<redacted>)");

    let other_authority = Authority::new_session()?;
    let other_token = other_authority.issue_root(id, &scope(resource), &TestClock)?;
    assert!(!binding.matches_token(&other_token));
    Ok(())
}

#[test]
fn child_binding_preserves_root_identity() -> Result<(), Box<dyn std::error::Error>> {
    let root_id = FrameId::new_v7();
    let (mut root, root_token) = active_frame(root_id, scope(ResourceScope::Workspace))?;
    let child_scope = scope(ResourceScope::Prefix(ResourcePath::new("src")?));
    let child = root.spawn_child(
        AgentRole::SubAgent,
        child_scope,
        ResourceQuota {
            cpu_millis: 1_000,
            memory_mb: 128,
            network: Default::default(),
            max_steps: 2,
            max_depth: 2,
            token_budget: 100,
        },
        ScratchpadRef::new(Cid::from_bytes([10; 32])),
        &TestClock,
    )?;
    let child = child.activate(
        ModelSelection {
            model: polaris_sdk::ids::ModelId::new("test/model")?,
            maximum_effort: Effort::Low,
        },
        Effort::Low,
    )?;
    let child = child.suspend().resume();
    let child_binding = child.tool_binding();
    assert_ne!(child_binding.frame_id(), root_id);
    assert_eq!(child_binding.root_frame_id(), root_id);
    assert!(!child_binding.matches_token(&root_token));
    Ok(())
}
