//! Cold stdio contracts preserve secret redaction and enum dispatch.
use polaris_sdk::{
    backends::{OneOf2, OneOf3},
    interactive::{InteractiveSandbox, InteractiveSession, StdioRequest},
    registry::{SandboxError, SandboxSpec},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct RefusingLauncher(Arc<AtomicUsize>, usize);
impl InteractiveSandbox for RefusingLauncher {
    async fn launch_stdio(
        &self,
        _spec: &SandboxSpec,
        request: &StdioRequest,
    ) -> Result<InteractiveSession, SandboxError> {
        self.0.fetch_add(self.1, Ordering::SeqCst);
        if request
            .argv
            .first()
            .is_none_or(|value| value != "fixture-server")
        {
            return Err(SandboxError::Protocol(
                "dispatch changed launch input".into(),
            ));
        }
        Err(SandboxError::PolicyDenied("fixture refuses launch".into()))
    }
}
fn request() -> StdioRequest {
    StdioRequest {
        argv: vec!["fixture-server".into()],
        workdir: None,
        env: vec![("FIXTURE_KEY".into(), "synthetic-sensitive-canary".into())],
        timeout: None,
        output_limit_bytes: None,
    }
}
#[test]
fn interactive_debug_never_exposes_arguments_or_environment() {
    let mut launch = request();
    launch.argv.push("synthetic-argument-secret".into());
    let debug = format!("{launch:?}");
    assert!(!debug.contains("canary"));
    assert!(!debug.contains("synthetic-argument-secret"));
    assert!(!debug.contains("FIXTURE_KEY"));
}
#[tokio::test]
async fn interactive_enum_and_arc_preserve_dispatch_and_denial() {
    let calls = Arc::new(AtomicUsize::new(0));
    let one: OneOf2<RefusingLauncher, RefusingLauncher> =
        OneOf2::B(RefusingLauncher(Arc::clone(&calls), 2));
    let two: OneOf3<RefusingLauncher, RefusingLauncher, RefusingLauncher> =
        OneOf3::C(RefusingLauncher(Arc::clone(&calls), 3));
    let spec = SandboxSpec::default();
    assert!(matches!(
        Arc::new(one).launch_stdio(&spec, &request()).await,
        Err(SandboxError::PolicyDenied(_))
    ));
    assert!(matches!(
        two.launch_stdio(&spec, &request()).await,
        Err(SandboxError::PolicyDenied(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 5);
}
