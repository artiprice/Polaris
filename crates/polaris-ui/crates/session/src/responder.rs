//! Model replies for a session hosted by `polarisd` (P9). The daemon answers each typed message
//! of its session with a chat turn on its own kernel; frontends attached over RPC read the reply
//! back from the step's `FrameStepRecorded` record ([`crate::RemoteSession`]).
//!
//! Nobody attached can review a change, so the turns offer only the read tools and show changes
//! as patches. Slash commands stay with the frontends, which run them on their own workspace.

use crate::{
    ChatAgent, ChatError, ChatSettings, chat::skipped_reasons, choose_provider,
    commands::LocalCommand,
};
use polaris_kernel::KernelHandle;
use polaris_sdk::{
    contracts::Event,
    ids::{Cid, SessionId},
};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Messages waiting for a reply; later ones are dropped (and reported) while the queue is full.
const QUEUE: usize = 32;
/// Largest message text read back from the content store.
const MAX_INPUT_BYTES: usize = 65_536;

/// Provider configuration of the replies.
#[derive(Debug, Clone)]
pub struct ReplyConfig {
    /// Layered configuration (provider order and models).
    pub config: polaris_config::PolarisConfig,
    /// Dotenv file with provider keys; a repository `.env` is never read implicitly.
    pub env_file: Option<PathBuf>,
}

/// Answer every message typed into `session` from now on until `stop`, one turn at a time, on
/// `kernel` over `workspace`. `log` receives one sanitized line per event worth reporting.
/// # Errors
/// No usable provider, an unreadable dotenv file, or agent setup failure.
pub async fn answer_session_input(
    kernel: KernelHandle,
    session: SessionId,
    workspace: PathBuf,
    reply: ReplyConfig,
    stop: CancellationToken,
    mut log: impl FnMut(String) + Send,
) -> Result<(), ChatError> {
    // Subscribed before the provider is chosen, so a message typed meanwhile is still answered.
    let (queued, mut inputs) = mpsc::channel::<Cid>(QUEUE);
    let (dropped_tx, mut dropped) = mpsc::unbounded_channel::<()>();
    let mut events = kernel.context().subscribe();
    let reader_stop = stop.clone();
    let reader = tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                () = reader_stop.cancelled() => return,
                next = events.next() => next,
            };
            match next {
                Ok(record) => {
                    if record.session() != session {
                        continue;
                    }
                    if let Event::UserInputSubmitted { text } = record.event()
                        && queued.try_send(*text).is_err()
                    {
                        let _reported = dropped_tx.send(());
                    }
                }
                Err(polaris_kernel::bus::BusError::Lagged { .. }) => {
                    let _reported = dropped_tx.send(());
                }
                Err(_) => return,
            }
        }
    });
    let _reader = AbortOnDrop(reader);

    let settings = ChatSettings::default();
    let choice = choose_provider(&reply.config, reply.env_file.as_deref(), settings.effort).await?;
    let reasons = skipped_reasons(&choice);
    let Some(selected) = choice.selected else {
        return Err(ChatError::NoProvider(reasons));
    };
    let model_id = selected.model.model().clone();
    let context_tokens = selected.model.max_context_tokens().get();
    log(format!(
        "replies from {} ({})",
        selected.provider.as_str(),
        model_id.as_str()
    ));
    let mut agent = ChatAgent::new(
        &kernel,
        &workspace,
        model_id,
        Arc::new(selected.model),
        context_tokens,
        settings,
    )
    .await?;
    agent.set_read_only(true);

    loop {
        let text = tokio::select! {
            biased;
            () = stop.cancelled() => break,
            Some(()) = dropped.recv() => {
                log("a message arrived while the reply queue was full; it is not answered".to_owned());
                continue;
            }
            next = inputs.recv() => match next {
                Some(text) => text,
                None => break,
            },
        };
        let Some(prompt) = read_input(&kernel, text) else {
            log("a message could not be read back from the content store".to_owned());
            continue;
        };
        if LocalCommand::parse(&prompt).is_some() {
            continue;
        }
        // Nobody consumes live progress here; the reply reaches frontends through the log.
        let (progress, receiver) = mpsc::channel(1);
        drop(receiver);
        match agent.turn(&kernel, &prompt, &stop, progress).await {
            Ok(turn) => log(format!(
                "replied: {} request(s), {} tool round(s), {}",
                turn.attempts, turn.tool_rounds, turn.stop
            )),
            Err(ChatError::Cancelled) => break,
            Err(error) => log(format!("reply failed: {error}")),
        }
    }
    Ok(())
}

/// The message text `cid` names, when it is in the content store and valid UTF-8.
fn read_input(kernel: &KernelHandle, cid: Cid) -> Option<String> {
    use polaris_kernel::verifier::EvidenceStore;
    let bytes = polaris_kernel::KernelEvidence::new(Arc::clone(kernel.context()))
        .get(&cid)
        .ok()??;
    if bytes.len() > MAX_INPUT_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
