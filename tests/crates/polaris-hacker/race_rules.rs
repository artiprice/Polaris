//! Lexical race rules of `static_scan`: split lock scopes and atomic load/store pairs, with the
//! negatives that must stay quiet (one guard held, read-modify-write, comments and literals).

use polaris_code_intel::Lang;
use polaris_hacker::{ATOMIC_LOAD_STORE, FindingClass, SPLIT_LOCK, SourceFile, static_scan};
use polaris_security_automata::DfaScanner;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn races(source: &str) -> Result<Vec<(String, usize)>, Box<dyn std::error::Error>> {
    let file = SourceFile {
        path: "src/lib.rs".to_owned(),
        lang: Lang::Rust,
        bytes: source.as_bytes().to_vec(),
    };
    let rules = DfaScanner::builder().build(&Default::default())?;
    let report = static_scan(&[file], &rules)?;
    Ok(report
        .findings
        .into_iter()
        .filter(|finding| finding.class == FindingClass::RaceCandidate)
        .map(|finding| {
            let line = source
                .get(..finding.start)
                .map_or(0, |prefix| prefix.matches('\n').count() + 1);
            (finding.rule, line)
        })
        .collect())
}

#[test]
fn a_lock_released_between_check_and_write_is_a_split_lock() -> TestResult {
    let found = races(
        "fn withdraw(a: &Account, n: u64) -> bool {\n    let balance = *a.balance.lock().unwrap();\n    if balance < n { return false; }\n    *a.balance.lock().unwrap() = balance - n;\n    true\n}\n",
    )?;
    assert_eq!(found, [(SPLIT_LOCK.to_owned(), 4)]);
    // A condition's temporary guard is gone before the body runs.
    let found = races(
        "fn insert(c: &Cache, k: &str) {\n    if c.map.lock().unwrap().contains_key(k) { return; }\n    c.map.lock().unwrap().insert(k.into(), 1);\n}\n",
    )?;
    assert_eq!(found, [(SPLIT_LOCK.to_owned(), 3)]);
    // RwLock: checked under read, written under write.
    let found = races(
        "fn set(s: &State) {\n    let empty = s.items.read().unwrap().is_empty();\n    if empty { s.items.write().unwrap().push(1); }\n}\n",
    )?;
    assert_eq!(found, [(SPLIT_LOCK.to_owned(), 3)]);
    Ok(())
}

#[test]
fn one_held_guard_is_not_a_split_lock_unless_dropped_first() -> TestResult {
    let held = races(
        "fn withdraw(a: &Account, n: u64) -> bool {\n    let mut balance = a.balance.lock().unwrap();\n    if *balance < n { return false; }\n    *balance -= n;\n    true\n}\n",
    )?;
    assert!(held.is_empty(), "{held:?}");
    let dropped = races(
        "fn withdraw(a: &Account, n: u64) {\n    let guard = a.balance.lock().unwrap();\n    let seen = *guard;\n    drop(guard);\n    *a.balance.lock().unwrap() = seen - n;\n}\n",
    )?;
    assert_eq!(dropped, [(SPLIT_LOCK.to_owned(), 5)]);
    // Different locks, or the same lock in different functions, are unrelated.
    let separate = races(
        "fn a(s: &S) { let x = *s.left.lock().unwrap(); *s.right.lock().unwrap() = x; }\nfn b(s: &S) -> u64 { *s.left.lock().unwrap() }\nfn c(s: &S) { *s.left.lock().unwrap() = 0; }\n",
    )?;
    assert!(separate.is_empty(), "{separate:?}");
    Ok(())
}

/// The shapes that made the first version noisy on real code (P7 calibration on polaris-kernel).
#[test]
fn writes_bound_guards_and_unrelated_methods_are_not_checks() -> TestResult {
    // Two writes in a row check nothing.
    let writes = races(
        "fn sanitize(s: &S) -> Result<(), E> {\n    *s.report.lock().map_err(|_| E)? = None;\n    let report = run()?;\n    *s.report.lock().map_err(|_| E)? = Some(report);\n    Ok(())\n}\n",
    )?;
    assert!(writes.is_empty(), "{writes:?}");
    // Guards bound by `if let Ok(..)` live through their blocks; separate operations follow.
    let bound = races(
        "fn run(s: &S, id: u64) {\n    if let Ok(mut pending) = s.pending.lock() && let Some(r) = pending.remove(&id) { r.send(); }\n    work();\n    if let Ok(mut pending) = s.pending.lock() { pending.clear(); }\n}\n",
    )?;
    assert!(bound.is_empty(), "{bound:?}");
    // `read()`/`write()` of something that is not a lock (no unwrap/expect/await).
    let methods = races(
        "fn t(f: &Fixture) -> R {\n    let before = f.read()?;\n    f.start()?;\n    let after = f.read()?;\n    check(before, after)\n}\n",
    )?;
    assert!(methods.is_empty(), "{methods:?}");
    // A condition far from the next lock is not its check.
    let far = races(
        "fn f(s: &S) {\n    if s.v.lock().unwrap().is_empty() { return; }\n    a();\n    b();\n    c();\n    d();\n    e();\n    g();\n    h();\n    i();\n    *s.v.lock().unwrap() = Vec::new();\n}\n",
    )?;
    assert!(far.is_empty(), "{far:?}");
    Ok(())
}

#[test]
fn an_atomic_load_then_store_loses_updates() -> TestResult {
    let found = races(
        "fn next() -> u64 {\n    let id = NEXT.load(Ordering::SeqCst);\n    NEXT.store(id + 1, Ordering::SeqCst);\n    id\n}\n",
    )?;
    assert_eq!(found, [(ATOMIC_LOAD_STORE.to_owned(), 3)]);
    let atomic = races(
        "fn next() -> u64 { NEXT.fetch_add(1, Ordering::SeqCst) }\nfn bump() -> bool {\n    let v = NEXT.load(Ordering::Acquire);\n    NEXT.compare_exchange(v, v + 1, Ordering::AcqRel, Ordering::Acquire).is_ok()\n}\nfn reset() { NEXT.store(0, Ordering::SeqCst); }\n",
    )?;
    assert!(atomic.is_empty(), "{atomic:?}");
    // A gate lock held across the load and the store serialises them.
    let gated = races(
        "fn resize(p: &Pool, n: u32) {
    let _gate = p.lock();
    let current = p.limit.load(Ordering::SeqCst);
    if current == n { return; }
    p.limit.store(n, Ordering::SeqCst);
}
",
    )?;
    assert!(gated.is_empty(), "{gated:?}");
    Ok(())
}

#[test]
fn comments_and_literals_are_not_code() -> TestResult {
    let found = races(
        "fn f(s: &S) {\n    // let x = *s.v.lock().unwrap(); *s.v.lock().unwrap() = 1;\n    let text = \"N.load(); N.store(1)\";\n    let raw = r#\"s.v.lock() s.v.lock()\"#;\n    let quote = '\\'';\n    /* s.v.lock(); /* nested */ s.v.lock(); */\n    let _ = (text, raw, quote);\n}\n",
    )?;
    assert!(found.is_empty(), "{found:?}");
    Ok(())
}
