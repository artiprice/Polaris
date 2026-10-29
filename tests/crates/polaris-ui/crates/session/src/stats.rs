// Unit tests of crates/polaris-ui/crates/session/src/stats.rs (`tests`); compiled into that module via #[path].
use super::{level, stats_lines};
use crate::SessionMeta;
use polaris_sdk::ids::SessionId;

fn row(at: &str, prompt: u64, completion: u64) -> Result<SessionMeta, Box<dyn std::error::Error>> {
    let at: jiff::Timestamp = at.parse()?;
    Ok(SessionMeta {
        id: SessionId::new_v7(),
        name: "s".to_owned(),
        workspace: "w".into(),
        branch: String::new(),
        base_commit: String::new(),
        prompt_tokens: prompt,
        completion_tokens: completion,
        sandbox_checkpoint: None,
        status: polaris_kernel::storage::registry::SessionStatus::Paused,
        created_at: at,
        updated_at: at,
    })
}

#[test]
fn stats_total_tokens_and_mark_the_busiest_day() -> Result<(), Box<dyn std::error::Error>> {
    let zone = jiff::tz::TimeZone::UTC;
    let rows = vec![
        row("2026-10-29T10:00:00Z", 1000, 200)?,
        row("2026-10-29T12:00:00Z", 10, 5)?,
        row("2026-10-27T09:00:00Z", 100, 50)?,
    ];
    let today = jiff::civil::date(2026, 10, 29);
    let lines = stats_lines(&rows, today, &zone);
    let text = lines.join("\n");
    assert!(text.contains("Sessions            3"), "{text}");
    assert!(text.contains("Prompt tokens       1110"), "{text}");
    assert!(
        text.contains("Busiest day         2026-10-29 · 1215 tokens"),
        "{text}"
    );
    assert!(
        text.contains("Thu 29  ████████████████████████ 1215"),
        "{text}"
    );
    assert!(text.contains("Less · ░ ▒ ▓ █ More"), "{text}");
    let thursday = lines
        .iter()
        .find(|line| line.starts_with("    ") && line.chars().filter(|c| *c == '█').count() == 1)
        .is_some();
    assert!(
        thursday,
        "the busiest day is the darkest calendar cell: {text}"
    );
    Ok(())
}

#[test]
fn levels_split_the_peak_into_quarters() {
    assert_eq!(level(0, 100), 0);
    assert_eq!(level(1, 100), 1);
    assert_eq!(level(26, 100), 2);
    assert_eq!(level(100, 100), 4);
    assert_eq!(level(5, 0), 0);
}
