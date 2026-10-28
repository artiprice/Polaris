use polaris_runtime_frames::{
    Active, AgentContextFrame, AgentRole, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
};
use polaris_sdk::{
    contracts::Timestamp,
    effort::Effort,
    ids::{Cid, FrameId, ModelId},
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

fn scope() -> Scope {
    Scope {
        resource: ResourceScope::Workspace,
        max_churn_lines: 100,
        ttl: Duration::from_secs(3_600),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read, Right::ExecuteTool]),
    }
}

fn active_frame(
    id: FrameId,
    role: AgentRole,
    model: &str,
    effort: Effort,
    maximum_effort: Effort,
) -> Result<(AgentContextFrame<Active>, Token), Box<dyn std::error::Error>> {
    let authority = Authority::new_session()?;
    let capability_scope = scope();
    let token = authority.issue_root(id, &capability_scope, &TestClock)?;
    let frame = AgentContextFrame::new_root(
        id,
        role,
        token.clone(),
        FrameScope {
            capability: capability_scope,
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
            model: ModelId::new(model)?,
            maximum_effort,
        },
        effort,
    )?;
    Ok((frame, token))
}

#[test]
fn prompt_binding_captures_the_active_frame_selection() -> Result<(), Box<dyn std::error::Error>> {
    let id = FrameId::new_v7();
    let (frame, _) = active_frame(
        id,
        AgentRole::Orchestrator,
        "test/selected-model",
        Effort::High,
        Effort::High,
    )?;

    let binding = frame.prompt_binding()?;

    assert_eq!(binding.frame_id(), id);
    assert_eq!(binding.root_frame_id(), id);
    assert_eq!(binding.role(), AgentRole::Orchestrator);
    assert_eq!(binding.model().as_str(), "test/selected-model");
    assert_eq!(binding.effort(), Effort::High);
    Ok(())
}

#[test]
fn prompt_binding_keeps_child_and_foreign_frame_identity_distinct()
-> Result<(), Box<dyn std::error::Error>> {
    let root_id = FrameId::new_v7();
    let (mut root, _) = active_frame(
        root_id,
        AgentRole::Orchestrator,
        "test/root-model",
        Effort::Low,
        Effort::Low,
    )?;
    let child_scope = Scope {
        resource: ResourceScope::Prefix(ResourcePath::new("src")?),
        max_churn_lines: 100,
        ttl: Duration::from_secs(3_600),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read]),
    };
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
            model: ModelId::new("test/child-model")?,
            maximum_effort: Effort::Medium,
        },
        Effort::Medium,
    )?;
    let foreign_id = FrameId::new_v7();
    let (foreign, _) = active_frame(
        foreign_id,
        AgentRole::Researcher,
        "test/foreign-model",
        Effort::Medium,
        Effort::Medium,
    )?;

    let root_binding = root.prompt_binding()?;
    let child_binding = child.prompt_binding()?;
    let foreign_binding = foreign.prompt_binding()?;

    assert_eq!(root_binding.root_frame_id(), root_id);
    assert_ne!(child_binding.frame_id(), root_id);
    assert_eq!(child_binding.root_frame_id(), root_id);
    assert_eq!(child_binding.role(), AgentRole::SubAgent);
    assert_eq!(child_binding.model().as_str(), "test/child-model");
    assert_eq!(child_binding.effort(), Effort::Medium);
    assert_eq!(foreign_binding.root_frame_id(), foreign_id);
    assert_ne!(root_binding.frame_id(), foreign_binding.frame_id());
    assert_ne!(child_binding.frame_id(), foreign_binding.frame_id());
    assert_ne!(root_binding.model(), foreign_binding.model());
    Ok(())
}
