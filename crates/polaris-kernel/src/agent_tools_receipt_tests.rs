//! Actual durable event/CAS lookup regressions; no sandbox or model execution is simulated here.
use super::*;
use crate::{ExecutionReview, KernelCtx};
use polaris_sdk::ids::{FrameId, SessionId};

type TestResult = Result<(), Box<dyn std::error::Error>>;
struct Fixture {
    _root: tempfile::TempDir,
    context: Arc<KernelCtx>,
    session: SessionId,
    frame: FrameId,
    review: ExecutionReview,
}
impl Fixture {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let context = KernelCtx::open(root.path()).await?;
        let session = SessionId::new_v7();
        let frame = FrameId::new_v7();
        let review = ExecutionReview {
            digest: context.cas.put(b"review-fixture")?,
            tree: context.cas.put(b"tree-fixture")?,
            details: json!({"frame":frame}),
        };
        Ok(Self {
            _root: root,
            context,
            session,
            frame,
            review,
        })
    }
    fn start(&self) -> TestResult {
        self.context.publish(
            self.session,
            Event::ToolExecutionStarted {
                frame: self.frame,
                call: self.review.digest,
            },
        )?;
        Ok(())
    }
    fn report(
        &self,
        frame: FrameId,
        review: Cid,
        passed: bool,
    ) -> Result<Cid, Box<dyn std::error::Error>> {
        let report=self.context.cas.put(&serde_json::to_vec(&json!({"schema":2,"session":self.session,"frame":frame,"review":review,"tree":self.review.tree,"passed":passed}))?)?;
        self.context.publish(
            self.session,
            if passed {
                Event::ProofOfDonePassed { frame, report }
            } else {
                Event::ProofOfDoneBlocked { frame, report }
            },
        )?;
        Ok(report)
    }
    fn complete(&self, ok: bool) -> TestResult {
        self.context.publish(
            self.session,
            Event::ToolExecutionCompleted {
                frame: self.frame,
                call: self.review.digest,
                ok,
                latency_us: 0,
            },
        )?;
        Ok(())
    }
    fn read(&self) -> Result<Option<(bool, Cid)>, ToolError> {
        read_test_receipt(&self.context, self.session, self.frame, &self.review)
    }
}
#[tokio::test]
async fn test_receipt_rejects_foreign_frame() -> TestResult {
    let f = Fixture::new().await?;
    f.start()?;
    f.report(FrameId::new_v7(), f.review.digest, true)?;
    f.complete(true)?;
    assert!(
        f.read().is_err(),
        "a foreign frame's proof cannot certify this execution"
    );
    Ok(())
}
#[tokio::test]
async fn test_receipt_rejects_wrong_review() -> TestResult {
    let f = Fixture::new().await?;
    f.start()?;
    f.report(f.frame, Cid::from_bytes([2; 32]), true)?;
    f.complete(true)?;
    assert!(
        f.read().is_err(),
        "proof must name the exact immutable review"
    );
    Ok(())
}
#[tokio::test]
async fn test_receipt_requires_matching_terminal_status() -> TestResult {
    let f = Fixture::new().await?;
    f.start()?;
    f.report(f.frame, f.review.digest, false)?;
    f.complete(true)?;
    assert!(
        f.read().is_err(),
        "failed proof cannot become a successful tool result"
    );
    Ok(())
}
#[tokio::test]
async fn test_receipt_accepts_exact_pass_and_cleanup_failure() -> TestResult {
    let f = Fixture::new().await?;
    assert_eq!(f.read()?, None, "unstarted review remains pending");
    f.start()?;
    let report = f.report(f.frame, f.review.digest, true)?;
    assert_eq!(
        f.read()?,
        None,
        "proof without durable completion remains pending"
    );
    f.complete(true)?;
    assert_eq!(f.read()?, Some((true, report)));
    let f = Fixture::new().await?;
    f.start()?;
    let report=f.context.cas.put(&serde_json::to_vec(&json!({"stage":"sandbox_cleanup","review":f.review.digest,"reason":"fixture cleanup failure"}))?)?;
    f.context.publish(
        f.session,
        Event::ProofOfDoneBlocked {
            frame: f.frame,
            report,
        },
    )?;
    f.complete(false)?;
    assert_eq!(f.read()?, Some((false, report)));
    Ok(())
}
