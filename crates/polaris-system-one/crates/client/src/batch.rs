use std::sync::Arc;
use std::time::Duration;

use polaris_proto::pb;
use polaris_proto::worker_auth::WorkerCredential;
use polaris_sdk::registry::DecisionError;
use polaris_sdk::system_one::{Decision, Question};
use prost::Message;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tonic::Request;

use crate::cache::{DecisionCache, ModelIdentity};
use crate::{ClientConfig, status_error, validate_response, worker_client};

pub(crate) const BATCH_WINDOW: Duration = Duration::from_millis(3);
pub(crate) const MAX_BATCH_ITEMS: usize = 8;
pub(crate) const QUEUE_CAPACITY: usize = 32;

pub(crate) struct PendingDecision {
    pub(crate) request: pb::DecisionRequest,
    pub(crate) schema: Vec<Question>,
    pub(crate) deadline: Instant,
    pub(crate) response: oneshot::Sender<Result<Decision, DecisionError>>,
}

pub(crate) async fn run(
    mut receiver: mpsc::Receiver<PendingDecision>,
    channel: tonic::transport::Channel,
    config: ClientConfig,
    credential: Arc<WorkerCredential>,
    cache: Arc<DecisionCache>,
) {
    let mut carry = None;
    loop {
        let Some(first) = (match carry.take() {
            Some(item) => Some(item),
            None => receiver.recv().await,
        }) else {
            return;
        };
        let mut pending = vec![first];
        let window_deadline = match Instant::now().checked_add(BATCH_WINDOW) {
            Some(deadline) => deadline,
            None => Instant::now(),
        };
        while pending.len() < MAX_BATCH_ITEMS {
            match tokio::time::timeout_at(window_deadline, receiver.recv()).await {
                Ok(Some(next)) => {
                    let mut prospective = pending
                        .iter()
                        .map(|item| item.request.clone())
                        .collect::<Vec<_>>();
                    prospective.push(next.request.clone());
                    if (pb::DecideRequest { items: prospective }).encoded_len()
                        > polaris_proto::limits::MAX_MESSAGE_BYTES
                    {
                        carry = Some(next);
                        break;
                    }
                    pending.push(next);
                }
                Ok(None) | Err(_) => break,
            }
        }
        pending.retain(|item| !item.response.is_closed() && item.deadline > Instant::now());
        if pending.is_empty() {
            continue;
        }
        dispatch(&channel, config, &credential, &cache, pending).await;
    }
}

async fn dispatch(
    channel: &tonic::transport::Channel,
    config: ClientConfig,
    credential: &WorkerCredential,
    cache: &DecisionCache,
    mut pending: Vec<PendingDecision>,
) {
    let identity_before = loaded_model_identity(channel, config, credential, &pending).await;
    pending.retain(|item| !item.response.is_closed() && item.deadline > Instant::now());
    if pending.is_empty() {
        return;
    }
    let requests = pending.iter().map(|item| item.request.clone()).collect();
    let request = credential.authorize_request(Request::new(pb::DecideRequest { items: requests }));
    let mut client = worker_client(channel.clone());
    let response = match request {
        Err(error) => Err(status_error(error)),
        Ok(request) => {
            let remaining = pending
                .iter()
                .map(|item| item.deadline.saturating_duration_since(Instant::now()))
                .max();
            let remaining = match remaining {
                Some(value) => value,
                None => Duration::ZERO,
            };
            if remaining.is_zero() {
                Err(DecisionError::Timeout)
            } else {
                match tokio::time::timeout(remaining.min(config.timeout), client.decide(request))
                    .await
                {
                    Err(_) => Err(DecisionError::Timeout),
                    Ok(Err(error)) => Err(status_error(error)),
                    Ok(Ok(response)) => Ok(response.into_inner().decisions),
                }
            }
        }
    };
    match response {
        Err(error) => {
            for item in pending {
                let _ = item.response.send(Err(error.clone()));
            }
        }
        Ok(decisions) if decisions.len() != pending.len() => {
            for item in pending {
                let _ = item.response.send(Err(DecisionError::Model(
                    "worker returned an unexpected number of decisions".to_owned(),
                )));
            }
        }
        Ok(decisions) => {
            let identity_after = loaded_model_identity(channel, config, credential, &pending).await;
            let stable_identity = match (identity_before, identity_after) {
                (Some(before), Some(after)) if before == after => Some(before),
                _ => None,
            };
            if stable_identity.is_none() {
                cache.clear();
            }
            for (item, raw) in pending.into_iter().zip(decisions) {
                let result = Decision::try_from(raw)
                    .map_err(|_| {
                        DecisionError::Model("worker returned a malformed decision".to_owned())
                    })
                    .and_then(|decision| {
                        validate_response(&item.schema, &decision)?;
                        Ok(decision)
                    });
                if let (Some(identity), Ok(decision)) = (stable_identity, &result)
                    && !item.response.is_closed()
                {
                    cache.insert(identity, &item.request, decision.clone());
                }
                let _ = item.response.send(result);
            }
        }
    }
}

async fn loaded_model_identity(
    channel: &tonic::transport::Channel,
    config: ClientConfig,
    credential: &WorkerCredential,
    pending: &[PendingDecision],
) -> Option<ModelIdentity> {
    let remaining = pending
        .iter()
        .map(|item| item.deadline.saturating_duration_since(Instant::now()))
        .max()?;
    if remaining.is_zero() {
        return None;
    }
    let request = credential
        .authorize_request(Request::new(pb::HealthRequest {}))
        .ok()?;
    let mut client = worker_client(channel.clone());
    let response = tokio::time::timeout(
        remaining.min(config.timeout).min(Duration::from_millis(50)),
        client.health(request),
    )
    .await
    .ok()?
    .ok()?
    .into_inner();
    if !response.ready {
        return None;
    }
    let remote_version = response.protocol_version?;
    let remote_version = polaris_proto::ProtocolVersion::try_from(remote_version).ok()?;
    polaris_proto::negotiate(polaris_proto::PROTOCOL_VERSION, remote_version).ok()?;
    ModelIdentity::from_health(&response).ok()?
}
