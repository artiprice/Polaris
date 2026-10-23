//! Repository skill approval persistence through the real redb-backed exact cache.

use std::error::Error;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use polaris_ext_skills::{SkillApprovalCache, SkillName};
use polaris_sdk::contracts::{ApprovalWitness, ApproverId, Timestamp};
use polaris_sdk::ids::Cid;
use polaris_storage::eventlog::{EventLog, EventLogConfig};

type TestResult = Result<(), Box<dyn Error>>;

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
    fn new() -> Result<Self, std::io::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)?;
        let name = format!(
            "polaris-skill-approval-{}-{}.redb",
            std::process::id(),
            now.as_nanos()
        );
        Ok(Self(std::env::temp_dir().join(name)))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn approval_cache_survives_eventlog_reopen_and_rejects_changed_evidence() -> TestResult {
    let database = TemporaryDatabase::new()?;
    let workspace = Cid::from_bytes([0x3a; 32]);
    let name = SkillName::new("repo-skill")?;
    let evidence = Cid::from_bytes([0x61; 32]);
    let now = Timestamp::from_microsecond(2_000_000_000_000_000)?;
    let approval = ApprovalWitness::new(ApproverId::new("trusted-reviewer")?, evidence, now);

    {
        let log = EventLog::open(EventLogConfig::new(database.path()))?;
        let cache = SkillApprovalCache::new(log, workspace);
        cache.save(&name, evidence, &approval)?;
    }

    let log = EventLog::open(EventLogConfig::new(database.path()))?;
    let cache = SkillApprovalCache::new(log, workspace);
    assert_eq!(cache.load(&name, evidence, now)?, Some(approval));
    assert_eq!(cache.load(&name, Cid::from_bytes([0x62; 32]), now)?, None);
    Ok(())
}
