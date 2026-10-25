//! Globally ordered replay projected into session-local views.
use crate::error::{ProjectionResource, ReduceError};
use crate::reduce::fold_unsequenced;
use crate::view::{MAX_SESSION_VIEWS, ViewState};
use polaris_sdk::contracts::EventRecord;
use polaris_sdk::effort::Effort;
use polaris_sdk::ids::{ModelId, Seq, SessionId};
use std::collections::BTreeMap;

/// Validates the single bus sequence before projecting records into independent sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionViews {
    model: ModelId,
    effort: Effort,
    last_seq: Option<Seq>,
    views: BTreeMap<SessionId, ViewState>,
}
impl SessionViews {
    /// Create an empty projection with shared model and effort defaults.
    #[must_use]
    pub fn new(model: ModelId, effort: Effort) -> Self {
        Self {
            model,
            effort,
            last_seq: None,
            views: BTreeMap::new(),
        }
    }
    #[must_use]
    /// Last globally ordered record accepted by this projection.
    pub const fn last_seq(&self) -> Option<Seq> {
        self.last_seq
    }
    #[must_use]
    /// Read a retained view for one session.
    pub fn get(&self, session: SessionId) -> Option<&ViewState> {
        self.views.get(&session)
    }
    /// Mutably access a retained view, for appending locally resolved display facts.
    pub fn get_mut(&mut self, session: SessionId) -> Option<&mut ViewState> {
        self.views.get_mut(&session)
    }
    #[must_use]
    /// Number of retained session views.
    pub fn len(&self) -> usize {
        self.views.len()
    }
    #[must_use]
    /// Whether no session view has been retained.
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
    /// Iterate over retained session IDs and their views in ID order.
    pub fn iter(&self) -> impl Iterator<Item = (SessionId, &ViewState)> {
        self.views.iter().map(|(session, view)| (*session, view))
    }
    fn validate_next(&self, record: &EventRecord) -> Result<(), ReduceError> {
        let seq = record.seq();
        if let Some(last) = self.last_seq {
            if seq <= last {
                return Err(ReduceError::OutOfOrder { last, got: seq });
            }
            match last.next() {
                Some(expected) if expected == seq => {}
                Some(expected) => return Err(ReduceError::SeqGap { expected, got: seq }),
                None => return Err(ReduceError::OutOfOrder { last, got: seq }),
            }
        }
        Ok(())
    }
    /// Apply one bus record after checking its position in the global stream.
    ///
    /// # Errors
    /// Returns sequence, capacity, or reducer errors without advancing the global cursor.
    pub fn apply(&mut self, record: &EventRecord) -> Result<(), ReduceError> {
        self.validate_next(record)?;
        let seq = record.seq();
        let session = record.session();
        if !self.views.contains_key(&session) && self.views.len() >= MAX_SESSION_VIEWS {
            return Err(ReduceError::CapacityExceeded {
                resource: ProjectionResource::SessionViews,
                limit: MAX_SESSION_VIEWS,
            });
        }
        let model = self.model.clone();
        let effort = self.effort;
        let view = self
            .views
            .entry(session)
            .or_insert_with(|| ViewState::new(model, effort));
        fold_unsequenced(view, record)?;
        self.last_seq = Some(seq);
        Ok(())
    }
    /// Validate every record against the global sequence but retain facts only for selected session.
    ///
    /// This is suitable for replaying the shared bus into one active session without allocating
    /// views for unrelated session IDs. Unrelated records still advance the global cursor.
    ///
    /// # Errors
    /// Returns sequence errors for any gap or out-of-order record, and capacity/allocation errors
    /// when a selected-session fact cannot be retained. Errors do not advance the global cursor.
    pub fn apply_selected(
        &mut self,
        record: &EventRecord,
        selected: SessionId,
    ) -> Result<(), ReduceError> {
        self.validate_next(record)?;
        let seq = record.seq();
        if record.session() == selected {
            if let Some(view) = self.views.get_mut(&selected) {
                fold_unsequenced(view, record)?;
            } else {
                if self.views.len() >= MAX_SESSION_VIEWS {
                    return Err(ReduceError::CapacityExceeded {
                        resource: ProjectionResource::SessionViews,
                        limit: MAX_SESSION_VIEWS,
                    });
                }
                let mut view = ViewState::new(self.model.clone(), self.effort);
                fold_unsequenced(&mut view, record)?;
                self.views.insert(selected, view);
            }
        }
        self.last_seq = Some(seq);
        Ok(())
    }
}
