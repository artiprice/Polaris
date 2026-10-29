//! `/stats`: activity across every local session, from registry rows only (no kernel).
use crate::SessionMeta;

/// Weeks shown in the activity calendar.
const CALENDAR_WEEKS: i64 = 12;
/// Intensity glyphs from no activity to the busiest day.
const LEVELS: [&str; 5] = ["·", "░", "▒", "▓", "█"];

/// One local calendar day's totals.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Day {
    sessions: u64,
    tokens: u64,
}

/// Render the stats sheet for `rows` as seen on `today` in `zone`.
#[must_use]
pub fn stats_lines(
    rows: &[SessionMeta],
    today: jiff::civil::Date,
    zone: &jiff::tz::TimeZone,
) -> Vec<String> {
    let mut days: std::collections::BTreeMap<jiff::civil::Date, Day> =
        std::collections::BTreeMap::new();
    let mut prompt = 0_u64;
    let mut completion = 0_u64;
    for row in rows {
        prompt = prompt.saturating_add(row.prompt_tokens);
        completion = completion.saturating_add(row.completion_tokens);
        let date = row.updated_at.to_zoned(zone.clone()).date();
        let day = days.entry(date).or_default();
        day.sessions = day.sessions.saturating_add(1);
        day.tokens = day
            .tokens
            .saturating_add(row.prompt_tokens.saturating_add(row.completion_tokens));
    }
    let mut lines = vec![
        format!("Sessions            {}", rows.len()),
        format!("Prompt tokens       {prompt}"),
        format!("Completion tokens   {completion}"),
        format!("Active days         {}", days.len()),
    ];
    if let Some((date, day)) = days.iter().max_by_key(|(_, day)| day.tokens) {
        if day.tokens > 0 {
            lines.push(format!(
                "Busiest day         {date} · {} tokens",
                day.tokens
            ));
        }
    }
    lines.push(String::new());
    lines.push("Last 7 days (tokens)".to_owned());
    let week_peak = (0..7)
        .filter_map(|back| today.checked_sub(jiff::Span::new().days(back)).ok())
        .map(|date| days.get(&date).map_or(0, |day| day.tokens))
        .max()
        .unwrap_or(0);
    for back in (0..7).rev() {
        let Ok(date) = today.checked_sub(jiff::Span::new().days(back)) else {
            continue;
        };
        let tokens = days.get(&date).map_or(0, |day| day.tokens);
        let width = if week_peak == 0 {
            0
        } else {
            usize::try_from(tokens.saturating_mul(24) / week_peak).unwrap_or(24)
        };
        lines.push(format!(
            "  {} {:>2}  {:<24} {tokens}",
            weekday(date.weekday()),
            date.day(),
            "█".repeat(width)
        ));
    }
    lines.push(String::new());
    lines.extend(calendar(&days, today));
    lines.push(String::new());
    lines
        .push("Counts come from the local session registry; /usage shows this session.".to_owned());
    lines
}

/// Monday-first heatmap of the last [`CALENDAR_WEEKS`] weeks, one column per week.
fn calendar(
    days: &std::collections::BTreeMap<jiff::civil::Date, Day>,
    today: jiff::civil::Date,
) -> Vec<String> {
    let monday = today
        .checked_sub(jiff::Span::new().days(i64::from(today.weekday().to_monday_zero_offset())))
        .unwrap_or(today);
    let start = monday
        .checked_sub(jiff::Span::new().weeks(CALENDAR_WEEKS - 1))
        .unwrap_or(monday);
    let peak = days
        .range(start..=today)
        .map(|(_, day)| day.tokens.max(day.sessions))
        .max()
        .unwrap_or(0);
    let mut lines = vec![format!("Activity, last {CALENDAR_WEEKS} weeks")];
    for row in 0..7_i64 {
        let mut line = format!("  {} ", weekday_short(row));
        for week in 0..CALENDAR_WEEKS {
            let date = start.checked_add(jiff::Span::new().days(week * 7 + row));
            let glyph = match date {
                Ok(date) if date > today => " ",
                Ok(date) => {
                    let value = days
                        .get(&date)
                        .map_or(0, |day| day.tokens.max(day.sessions));
                    LEVELS[level(value, peak)]
                }
                Err(_) => " ",
            };
            line.push_str(glyph);
            line.push(' ');
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.push(format!("  Less {} More", LEVELS.join(" ")));
    lines
}

/// 0 for no activity, otherwise 1..=4 by quarter of the peak.
fn level(value: u64, peak: u64) -> usize {
    if value == 0 || peak == 0 {
        return 0;
    }
    let quarter = value.saturating_mul(4).div_ceil(peak);
    usize::try_from(quarter.clamp(1, 4)).unwrap_or(4)
}

const fn weekday(day: jiff::civil::Weekday) -> &'static str {
    match day {
        jiff::civil::Weekday::Monday => "Mon",
        jiff::civil::Weekday::Tuesday => "Tue",
        jiff::civil::Weekday::Wednesday => "Wed",
        jiff::civil::Weekday::Thursday => "Thu",
        jiff::civil::Weekday::Friday => "Fri",
        jiff::civil::Weekday::Saturday => "Sat",
        jiff::civil::Weekday::Sunday => "Sun",
    }
}

const fn weekday_short(row: i64) -> &'static str {
    match row {
        0 => "Mo",
        2 => "We",
        4 => "Fr",
        6 => "Su",
        _ => "  ",
    }
}

#[cfg(test)]
mod tests {
    use super::{level, stats_lines};
    use crate::SessionMeta;
    use polaris_sdk::ids::SessionId;

    fn row(
        at: &str,
        prompt: u64,
        completion: u64,
    ) -> Result<SessionMeta, Box<dyn std::error::Error>> {
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
            .find(|line| {
                line.starts_with("    ") && line.chars().filter(|c| *c == '█').count() == 1
            })
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
}
