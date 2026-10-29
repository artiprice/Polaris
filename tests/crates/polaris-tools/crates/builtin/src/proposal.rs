// Unit tests of crates/polaris-tools/crates/builtin/src/proposal.rs (`tests`); compiled into that module via #[path].
use super::{
    CasKernelProposalBackend, EditFileRequest, ProposalTokenSource, ProposalToolBackend,
    full_replacement_diff,
};
use polaris_sdk::{
    contracts::{Command, CommandBody, EventRecord, TokenBytes, Unverified},
    ids::{Cid, FrameId, Seq, SessionId},
    registry::{CommandReceipt, EventSource, KernelApi, KernelError, ToolContext},
};
use polaris_storage_cas::{Cas, CasConfig, content_id};
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll, Waker},
};

#[test]
fn full_replacement_patch_retains_crlf_and_missing_final_newline()
-> Result<(), Box<dyn std::error::Error>> {
    let patch = full_replacement_diff("src.rs", b"old\r\ntext", "new\ncontent")?;
    let result = super::apply_unified_patch(b"old\r\ntext", &patch)?;
    assert_eq!(result, b"new\r\ncontent");
    let empty = full_replacement_diff("src.rs", b"", "created")?;
    assert_eq!(super::apply_unified_patch(b"", &empty)?, b"created");
    Ok(())
}

#[derive(Debug, Default)]
struct CapturedKernel(Mutex<Option<Cid>>);

#[derive(Debug)]
struct EmptyEvents;

impl EventSource for EmptyEvents {
    async fn next_event(&mut self) -> Option<EventRecord> {
        None
    }
}

impl KernelApi for CapturedKernel {
    type Events = EmptyEvents;

    async fn submit(&self, command: Command<Unverified>) -> Result<CommandReceipt, KernelError> {
        let CommandBody::ProposeDiff { diff } = *command.body() else {
            return Err(KernelError::Rejected("unexpected command".to_owned()));
        };
        *self
            .0
            .lock()
            .map_err(|_| KernelError::Internal("capture lock poisoned".to_owned()))? = Some(diff);
        Ok(CommandReceipt {
            command: command.id(),
            accepted_at: Seq::new(1),
        })
    }

    async fn subscribe(&self, _from: Seq) -> Result<Self::Events, KernelError> {
        Ok(EmptyEvents)
    }
}

#[derive(Debug)]
struct FixedToken;

impl ProposalTokenSource for FixedToken {
    fn token_for<'a>(
        &'a self,
        _session: SessionId,
        _frame: FrameId,
    ) -> Pin<Box<dyn Future<Output = Result<TokenBytes, KernelError>> + Send + 'a>> {
        Box::pin(async { Ok(TokenBytes::from_static(b"test-only-token")) })
    }
}

#[test]
fn edit_submits_full_occ_bound_diff_and_never_writes_host_file()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let storage = tempfile::tempdir()?;
    let path = workspace.path().join("sample.txt");
    let original = b"first\r\nold\r\nlast";
    std::fs::write(&path, original)?;
    let cas = Cas::open(CasConfig::new(storage.path().join("cas")))?;
    let kernel = CapturedKernel::default();
    let backend = CasKernelProposalBackend::new(cas.clone(), kernel, FixedToken);
    let context = ToolContext {
        session: SessionId::from_u128(11),
        frame: FrameId::from_u128(12),
        workspace_root: workspace.path(),
        deadline: None,
    };
    let limits = crate::ToolLimits {
        max_read_bytes: 1024,
        max_results: 16,
        max_output_bytes: 4096,
        max_input_bytes: 4096,
        regex_size_limit: 1024,
        timeout: std::time::Duration::from_secs(1),
    };
    let result = block_on(backend.edit_file(
        &context,
        EditFileRequest {
            path: "sample.txt",
            old_text: "old",
            new_text: "new",
            replace_all: false,
            expected_base: content_id(original),
        },
        &limits,
    ))?;
    assert_eq!(std::fs::read(&path)?, original);
    let diff_hex = result
        .get("diff_cid")
        .and_then(serde_json::Value::as_str)
        .ok_or("diff CID missing")?;
    let diff_cid = Cid::from_hex(diff_hex)?;
    let bytes = cas.get(&diff_cid)?;
    let diff = std::str::from_utf8(&bytes)?;
    assert!(diff.contains("-first\n-old\n-last\n"));
    assert!(diff.contains("+first\n+new\n+last\n"));
    Ok(())
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}
