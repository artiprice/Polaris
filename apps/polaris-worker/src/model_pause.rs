//! Monotonic authenticated host-pause epochs, owned by the native inference actor.

#[derive(Debug, Default)]
pub(crate) struct ModelPause {
    highest: u64,
    active: Option<u64>,
}

impl ModelPause {
    pub(crate) fn apply(&mut self, epoch: u64, resume: bool) -> Result<(), &'static str> {
        if epoch == 0 {
            return if resume {
                Err("resume requires a nonzero pause epoch")
            } else {
                Ok(())
            };
        }
        if resume {
            if epoch != self.highest {
                return Err("stale pause release rejected");
            }
            // An exact duplicate release is idempotent after a lost transport response.
            self.active = None;
            return Ok(());
        }
        if epoch < self.highest || (epoch == self.highest && self.active.is_none()) {
            return Err("stale pause acquisition rejected");
        }
        self.highest = epoch;
        self.active = Some(epoch);
        Ok(())
    }

    pub(crate) const fn paused(&self) -> bool {
        self.active.is_some()
    }
    pub(crate) const fn epoch(&self) -> u64 {
        self.highest
    }
}
