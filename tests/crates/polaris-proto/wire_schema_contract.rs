//! Wire Schema Contract (descriptor: specs/descriptors/proto_wire_schema_contract.json).

use std::collections::BTreeMap;
use std::error::Error;

use polaris_proto::{FILE_DESCRIPTOR_SET, pb};
use prost::Message;
use prost_types::field_descriptor_proto::Type;
use prost_types::{DescriptorProto, EnumDescriptorProto, FileDescriptorSet};

type TestResult = Result<(), Box<dyn Error>>;

const PACKAGE: &str = "polaris.v1";

fn set() -> Result<FileDescriptorSet, Box<dyn Error>> {
    Ok(FileDescriptorSet::decode(FILE_DESCRIPTOR_SET)?)
}

/// Top-level messages of package `polaris.v1`, by name.
fn messages(set: &FileDescriptorSet) -> BTreeMap<String, DescriptorProto> {
    let mut out = BTreeMap::new();
    for file in &set.file {
        if file.package() != PACKAGE {
            continue;
        }
        for message in &file.message_type {
            out.insert(message.name().to_owned(), message.clone());
        }
    }
    out
}

fn enums(set: &FileDescriptorSet) -> Vec<EnumDescriptorProto> {
    set.file
        .iter()
        .filter(|f| f.package() == PACKAGE)
        .flat_map(|f| f.enum_type.iter().cloned())
        .collect()
}

/// Resolves `Outer` or `Outer.Inner`.
fn find<'a>(
    all: &'a BTreeMap<String, DescriptorProto>,
    path: &str,
) -> Result<&'a DescriptorProto, Box<dyn Error>> {
    let mut parts = path.split('.');
    let first = parts.next().ok_or("empty path")?;
    let mut current = all
        .get(first)
        .ok_or_else(|| format!("no message {first}"))?;
    for part in parts {
        current = current
            .nested_type
            .iter()
            .find(|m| m.name() == part)
            .ok_or_else(|| format!("no nested message {part} in {path}"))?;
    }
    Ok(current)
}

fn field_table(message: &DescriptorProto) -> Vec<(String, i32)> {
    let mut fields: Vec<(String, i32)> = message
        .field
        .iter()
        .map(|f| (f.name().to_owned(), f.number()))
        .collect();
    fields.sort_by_key(|(_, number)| *number);
    fields
}

fn expected(rows: &[(&str, i32)]) -> Vec<(String, i32)> {
    rows.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect()
}

#[test]
fn descriptor_set_lists_the_three_services() -> TestResult {
    let set = set()?;
    let mut services: BTreeMap<String, Vec<(String, bool, bool)>> = BTreeMap::new();
    for file in set.file.iter().filter(|f| f.package() == PACKAGE) {
        for service in &file.service {
            let methods = service
                .method
                .iter()
                .map(|m| {
                    (
                        m.name().to_owned(),
                        m.client_streaming(),
                        m.server_streaming(),
                    )
                })
                .collect();
            services.insert(service.name().to_owned(), methods);
        }
    }
    let row = |name: &str, client: bool, server: bool| (name.to_owned(), client, server);
    let mut want: BTreeMap<String, Vec<(String, bool, bool)>> = BTreeMap::new();
    want.insert(
        "PolarisKernelService".to_owned(),
        vec![
            row("SubmitCommand", false, false),
            row("Subscribe", false, true),
            row("AttachTerminal", true, true),
            row("Health", false, false),
            // Protocol 1.3: attached frontends (P9).
            row("SubmitText", false, false),
            row("ReadText", false, false),
        ],
    );
    want.insert(
        "WorkerService".to_owned(),
        vec![
            row("Handshake", false, false),
            row("Decide", false, false),
            row("DetectPii", false, false),
            row("Embed", false, false),
            row("Ingest", false, false),
            row("EvictModels", false, false),
            row("Health", false, false),
            row("Shutdown", false, false),
        ],
    );
    want.insert(
        "GuestService".to_owned(),
        vec![
            row("Handshake", false, false),
            row("Exec", true, true),
            row("ApplySyncDelta", false, false),
            row("Screenshot", false, false),
            row("A11yTree", false, false),
            row("Health", false, false),
        ],
    );
    assert_eq!(services, want);
    Ok(())
}

#[test]
fn guest_handshake_keeps_the_spike_fields() -> TestResult {
    let set = set()?;
    let all = messages(&set);
    // Spike S0-12 printed: protocol_version, guest_version, os, arch, pid.
    assert_eq!(
        field_table(find(&all, "GuestHandshakeResponse")?),
        expected(&[
            ("protocol_version", 1),
            ("guest_version", 2),
            ("os", 3),
            ("arch", 4),
            ("pid", 5),
        ])
    );
    assert_eq!(
        field_table(find(&all, "GuestHandshakeRequest")?),
        expected(&[("protocol_version", 1)])
    );
    Ok(())
}

#[test]
fn field_numbers_are_frozen() -> TestResult {
    let set = set()?;
    let all = messages(&set);
    let table: Vec<(&str, Vec<(&str, i32)>)> = vec![
        ("Uuid", vec![("hi", 1), ("lo", 2)]),
        ("Cid", vec![("digest", 1)]),
        ("ProtocolVersion", vec![("major", 1), ("minor", 2)]),
        (
            "HealthResponse",
            vec![
                ("protocol_version", 1),
                ("build_version", 2),
                ("ready", 3),
                ("loaded_model_identity_blake3", 4),
                ("loaded_model_generation", 5),
                ("worker_incarnation", 6),
                ("loaded_pii_model_identity_blake3", 7),
                ("loaded_pii_model_generation", 8),
            ],
        ),
        (
            "Command",
            vec![("id", 1), ("frame", 2), ("token", 3), ("body", 4)],
        ),
        (
            "SubmitCommandResponse",
            vec![("command_id", 1), ("accepted_at", 2)],
        ),
        (
            "CommandBody",
            vec![
                ("submit_user_input", 1),
                ("propose_diff", 2),
                ("acquire_ast_scope", 3),
                ("release_ast_scope", 4),
                ("spawn_subagent", 5),
                ("execute_tool", 6),
                ("rollback_to", 7),
                ("cancel_frame", 8),
                ("apply_directive", 9),
                ("answer_human_request", 10),
            ],
        ),
        (
            "EventRecord",
            vec![
                ("seq", 1),
                ("at", 2),
                ("schema", 3),
                ("session", 4),
                ("event", 5),
            ],
        ),
        (
            "Event",
            vec![
                ("session_started", 1),
                ("directive_applied", 2),
                ("decision_made", 3),
                ("model_request_completed", 4),
                ("tool_execution_started", 5),
                ("tool_execution_completed", 6),
                ("diff_approved", 7),
                ("diff_rejected", 8),
                ("file_lease_granted", 9),
                ("file_lease_revoked", 10),
                ("sandbox_snapshot_branch_created", 11),
                ("sandbox_execution_completed", 12),
                ("proof_of_done_passed", 13),
                ("proof_of_done_blocked", 14),
                ("worker_restarted", 15),
                ("circuit_tripped", 16),
                ("dead_lettered", 17),
                ("session_closed", 18),
                ("session_configured", 19),
                ("session_paused", 20),
                ("session_resumed", 21),
                ("blast_radius_alert", 22),
                ("frame_step_recorded", 23),
                ("diff_needs_approval", 24),
                ("human_request_created", 25),
                ("human_request_answered", 26),
                ("human_request_interrupted", 27),
                ("frame_registered", 28),
                ("frame_tree_cancellation_requested", 29),
                ("frame_tool_result_recorded", 30),
                ("user_input_submitted", 31),
            ],
        ),
        (
            "Event.ModelRequestCompleted",
            vec![
                ("frame", 1),
                ("request", 2),
                ("prompt_tokens", 3),
                ("completion_tokens", 4),
                ("cached_tokens", 5),
                ("cost_micro_usd", 6),
                ("latency_us", 7),
            ],
        ),
        ("WorkerHandshakeRequest", vec![("protocol_version", 1)]),
        (
            "WorkerHandshakeResponse",
            vec![("protocol_version", 1), ("kind", 2), ("worker_version", 3)],
        ),
        ("Decision", vec![("answers", 1), ("latency_us", 2)]),
    ];
    for (name, rows) in table {
        assert_eq!(field_table(find(&all, name)?), expected(&rows), "{name}");
    }
    let receipt = find(&all, "SubmitCommandResponse")?;
    let sequence = receipt
        .field
        .iter()
        .find(|field| field.name() == "accepted_at")
        .ok_or("acceptance sequence field missing")?;
    assert!(
        sequence.proto3_optional(),
        "legacy missing sequence must differ from sequence zero"
    );
    Ok(())
}

#[test]
fn growing_messages_reserve_a_range() -> TestResult {
    let set = set()?;
    let all = messages(&set);
    for (name, message) in &all {
        let must_reserve = !message.oneof_decl.is_empty()
            || name.ends_with("Request")
            || name.ends_with("Response")
            || matches!(name.as_str(), "Command" | "EventRecord" | "Decision");
        let reserved: Vec<(i32, i32)> = message
            .reserved_range
            .iter()
            .map(|r| (r.start(), r.end()))
            .collect();
        if must_reserve {
            // Descriptor ranges are half-open: `reserved 100 to 199` is [100, 200).
            assert!(
                reserved.contains(&(100, 200)),
                "{name} lacks reserved 100..=199"
            );
        }
        for field in &message.field {
            for (start, end) in &reserved {
                assert!(
                    field.number() < *start || field.number() >= *end,
                    "{name}.{} uses a reserved number",
                    field.name()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn enums_start_with_unspecified() -> TestResult {
    let set = set()?;
    let all = enums(&set);
    let names: Vec<&str> = all.iter().map(|e| e.name()).collect();
    assert_eq!(
        names,
        ["AgentRole", "Effort", "WorkerKind", "FrameStepOutcome"]
    );
    for e in &all {
        let first = e.value.first().ok_or("enum without values")?;
        assert_eq!(first.number(), 0, "{}", e.name());
        assert!(first.name().ends_with("_UNSPECIFIED"), "{}", e.name());
    }
    // A default (all-zero) message can never decode to a real variant.
    assert_eq!(pb::AgentRole::default(), pb::AgentRole::Unspecified);
    assert_eq!(pb::Effort::default(), pb::Effort::Unspecified);
    assert_eq!(pb::WorkerKind::default(), pb::WorkerKind::Unspecified);
    assert_eq!(
        pb::FrameStepOutcome::default(),
        pb::FrameStepOutcome::Unspecified
    );
    Ok(())
}

#[test]
fn blobs_are_cid_references() -> TestResult {
    let set = set()?;
    let all = messages(&set);
    for outer in ["Event", "CommandBody"] {
        let message = find(&all, outer)?;
        assert!(
            !message.nested_type.is_empty(),
            "{outer} has payload messages"
        );
        for payload in &message.nested_type {
            for field in &payload.field {
                assert_ne!(
                    field.r#type,
                    Some(i32::from(Type::Bytes)),
                    "{outer}.{}.{} is inline bytes",
                    payload.name(),
                    field.name()
                );
            }
        }
    }
    // The only inline `bytes` in the whole package are the capability token and stream chunks.
    let mut inline: Vec<String> = Vec::new();
    for (name, message) in &all {
        for field in &message.field {
            if field.r#type == Some(i32::from(Type::Bytes)) {
                inline.push(format!("{name}.{}", field.name()));
            }
        }
    }
    inline.sort();
    assert_eq!(
        inline,
        [
            "Cid.digest",
            "Command.token",
            "ExecClientFrame.stdin",
            "ExecServerFrame.stderr",
            "ExecServerFrame.stdout",
            "HealthResponse.loaded_model_identity_blake3",
            "HealthResponse.loaded_pii_model_identity_blake3",
            "HealthResponse.worker_incarnation",
            "TerminalClientFrame.input",
            "TerminalServerFrame.output",
        ]
    );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn golden_bytes_are_stable() {
    // Synthetic fixtures (not captured from a running system): they pin the wire format.
    let cid = pb::Cid {
        digest: vec![0xAB; 32],
    };
    assert_eq!(
        hex(&cid.encode_to_vec()),
        format!("0a20{}", "ab".repeat(32))
    );

    let uuid = pb::Uuid { hi: 1, lo: 2 };
    assert_eq!(
        hex(&uuid.encode_to_vec()),
        "090100000000000000110200000000000000"
    );

    let record = pb::EventRecord {
        seq: 1,
        at: None,
        schema: 1,
        session: None,
        event: Some(pb::Event {
            kind: Some(pb::event::Kind::SessionClosed(pb::event::SessionClosed {})),
        }),
    };
    assert_eq!(hex(&record.encode_to_vec()), "080118012a03920100");

    let version = pb::ProtocolVersion { major: 1, minor: 0 };
    assert_eq!(hex(&version.encode_to_vec()), "0801");
}

#[test]
fn generated_stubs_exist_for_every_service() {
    use tonic::server::NamedService;

    let names = [
        std::any::type_name::<pb::polaris_kernel_service_client::PolarisKernelServiceClient<()>>(),
        std::any::type_name::<pb::worker_service_client::WorkerServiceClient<()>>(),
        std::any::type_name::<pb::guest_service_client::GuestServiceClient<()>>(),
    ];
    for name in names {
        assert!(name.contains("Client"), "{name}");
    }
    assert_eq!(
        <pb::polaris_kernel_service_server::PolarisKernelServiceServer<()> as NamedService>::NAME,
        "polaris.v1.PolarisKernelService"
    );
    assert_eq!(
        <pb::worker_service_server::WorkerServiceServer<()> as NamedService>::NAME,
        "polaris.v1.WorkerService"
    );
    assert_eq!(
        <pb::guest_service_server::GuestServiceServer<()> as NamedService>::NAME,
        "polaris.v1.GuestService"
    );
}
