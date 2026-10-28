//! Normalized, disjoint token categories. Demo records are not provider telemetry.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}
impl Default for Tokens {
    fn default() -> Self {
        Self {
            input: Some(0),
            output: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
        }
    }
}
impl Tokens {
    pub fn known(input: u64, output: u64, read: u64, write: u64) -> Self {
        Self {
            input: Some(input),
            output: Some(output),
            cache_read: Some(read),
            cache_write: Some(write),
        }
    }
    pub fn total(self) -> Option<u64> {
        self.input?
            .checked_add(self.output?)?
            .checked_add(self.cache_read?)?
            .checked_add(self.cache_write?)
    }
    pub fn add(&mut self, b: Self) {
        fn sum(a: Option<u64>, b: Option<u64>) -> Option<u64> {
            a?.checked_add(b?)
        }
        self.input = sum(self.input, b.input);
        self.output = sum(self.output, b.output);
        self.cache_read = sum(self.cache_read, b.cache_read);
        self.cache_write = sum(self.cache_write, b.cache_write);
    }
}
#[derive(Clone, Debug)]
pub struct Model {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct Usage {
    pub model: String,
    pub tokens: Tokens,
}
#[derive(Clone, Debug)]
pub struct Session {
    pub id: u64,
    pub day: i32,
    pub seconds: Option<u64>,
    pub usage: Vec<Usage>,
}
#[derive(Clone, Debug)]
pub struct Dataset {
    pub start: i32,
    pub end: i32,
    pub models: Vec<Model>,
    pub sessions: Vec<Session>,
}
#[derive(Clone, Debug)]
pub struct ModelTotal {
    pub id: String,
    pub name: String,
    pub tokens: Tokens,
    pub color: usize,
}
#[derive(Clone, Debug)]
pub struct Summary {
    pub tokens: Tokens,
    pub sessions: usize,
    pub active_days: usize,
    pub current_streak: usize,
    pub longest_streak: usize,
    pub longest_session: Option<u64>,
    pub seconds: Option<u64>,
    pub busiest: Option<(i32, usize)>,
    pub models: Vec<ModelTotal>,
}
#[derive(Clone, Debug)]
pub struct Bucket {
    pub start: i32,
    pub end: i32,
    pub models: Vec<Option<u64>>,
}
pub fn civil_day(y: i32, m: u32, d: u32) -> i32 {
    let y = y - i32::from(m <= 2);
    let era = y.div_euclid(400);
    let yo = y - era * 400;
    let mp = m as i32 + if m > 2 { -3 } else { 9 };
    era * 146097 + yo * 365 + yo / 4 - yo / 100 + (153 * mp + 2) / 5 + d as i32 - 1 - 719468
}
pub fn civil(day: i32) -> (i32, u32, u32) {
    let z = day + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yo = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yo + era * 400;
    let doy = doe - (365 * yo + yo / 4 - yo / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + i32::from(m <= 2), m as u32, d as u32)
}
pub fn weekday(day: i32) -> usize {
    (day + 3).rem_euclid(7) as usize
}
pub fn month(m: u32) -> &'static str {
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m.clamp(1, 12) - 1) as usize]
}
pub fn date(day: i32) -> String {
    let (y, m, d) = civil(day);
    format!("{} {d:02} '{:02}", month(m), y.rem_euclid(100))
}
pub fn short_date(day: i32) -> String {
    let (_, m, d) = civil(day);
    format!("{} {d:02}", month(m))
}
pub fn number(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{:.1}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}
pub fn count(n: Option<u64>) -> String {
    n.map(number).unwrap_or_else(|| "Unavailable".into())
}
pub fn duration(n: Option<u64>) -> String {
    n.map(|s| {
        if s >= 86400 {
            format!("{}d {}h", s / 86400, s % 86400 / 3600)
        } else if s >= 3600 {
            format!("{}h {}m", s / 3600, s % 3600 / 60)
        } else {
            format!("{}m {}s", s / 60, s % 60)
        }
    })
    .unwrap_or_else(|| "Unavailable".into())
}
impl Dataset {
    pub fn summary(&self, start: i32, end: i32) -> Summary {
        let rows = self
            .sessions
            .iter()
            .filter(|s| s.day >= start && s.day <= end)
            .collect::<Vec<_>>();
        let mut ids = BTreeSet::new();
        let mut days = BTreeMap::new();
        let mut tokens = Tokens::default();
        let mut totals = BTreeMap::<String, Tokens>::new();
        let mut seconds = Some(0u64);
        let mut longest = Some(0u64);
        for s in rows {
            if !ids.insert(s.id) {
                continue;
            }
            {
                *days.entry(s.day).or_insert(0usize) += 1;
                seconds = seconds.and_then(|a| s.seconds.and_then(|b| a.checked_add(b)));
                longest = longest.and_then(|a| s.seconds.map(|b| a.max(b)));
            }
            for u in &s.usage {
                tokens.add(u.tokens);
                totals.entry(u.model.clone()).or_default().add(u.tokens);
            }
        }
        let mut models = self
            .models
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                totals.get(&m.id).map(|t| ModelTotal {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    tokens: *t,
                    color: i,
                })
            })
            .collect::<Vec<_>>();
        for (id, tokens) in &totals {
            if !self.models.iter().any(|m| &m.id == id) {
                models.push(ModelTotal {
                    id: id.clone(),
                    name: crate::app::safe_text(id),
                    tokens: *tokens,
                    color: id
                        .bytes()
                        .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)),
                });
            }
        }
        models.sort_by(|a, b| {
            b.tokens
                .total()
                .cmp(&a.tokens.total())
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut current = 0;
        for day in (start..=end).rev() {
            if days.contains_key(&day) {
                current += 1
            } else {
                break;
            }
        }
        let (mut run, mut longest_streak) = (0, 0);
        for day in start..=end {
            if days.contains_key(&day) {
                run += 1;
                longest_streak = longest_streak.max(run);
            } else {
                run = 0;
            }
        }
        let busiest = days
            .iter()
            .max_by(|(da, ca), (db, cb)| ca.cmp(cb).then_with(|| db.cmp(da)))
            .map(|(d, c)| (*d, *c));
        Summary {
            tokens,
            sessions: ids.len(),
            active_days: days.len(),
            current_streak: current,
            longest_streak,
            longest_session: if ids.is_empty() { None } else { longest },
            seconds: if ids.is_empty() { None } else { seconds },
            busiest,
            models,
        }
    }
    pub fn buckets(
        &self,
        start: i32,
        end: i32,
        weekly: bool,
        models: &[ModelTotal],
    ) -> Vec<Bucket> {
        let mut result = vec![];
        let mut day = start;
        while day <= end {
            let last = if weekly {
                (day + 6 - weekday(day) as i32).min(end)
            } else {
                day
            };
            let summary = self.summary(day, last);
            let values = models
                .iter()
                .map(|m| {
                    if m.id == "__other" {
                        let mut t = Some(0u64);
                        for s in &summary.models {
                            if !models.iter().any(|known| known.id == s.id) {
                                t = t.and_then(|a| s.tokens.total().and_then(|b| a.checked_add(b)));
                            }
                        }
                        t
                    } else {
                        summary
                            .models
                            .iter()
                            .find(|s| s.id == m.id)
                            .map_or(Some(0), |s| s.tokens.total())
                    }
                })
                .collect();
            result.push(Bucket {
                start: day,
                end: last,
                models: values,
            });
            day = last + 1;
        }
        result
    }
}
pub fn chart_models(summary: &Summary) -> Vec<ModelTotal> {
    let mut models = summary.models.iter().take(5).cloned().collect::<Vec<_>>();
    if summary.models.len() > 5 {
        let mut tokens = Tokens::default();
        for m in summary.models.iter().skip(5) {
            tokens.add(m.tokens);
        }
        models.push(ModelTotal {
            id: "__other".into(),
            name: "Other".into(),
            tokens,
            color: usize::MAX,
        });
    }
    models
}
pub fn demo() -> &'static Dataset {
    static DATA: OnceLock<Dataset> = OnceLock::new();
    DATA.get_or_init(|| {
        let start = civil_day(2025, 10, 19);
        let models = [
            ("super", "Nemotron Super 49B v1.5"),
            ("nano9", "Nemotron Nano 9B v2"),
            ("nano8", "Nemotron Nano 8B v1"),
        ]
        .map(|(id, name)| Model {
            id: id.into(),
            name: name.into(),
        })
        .to_vec();
        let mut sessions = vec![];
        let mut id = 0;
        for i in 0..365 {
            if (i * 17 + i / 7 * 3 + 11) % 13 < 4 {
                continue;
            }
            for s in 0..1 + (i * 7 + 3) % 6 {
                id += 1;
                let mut usage = vec![];
                for (m, model) in models.iter().enumerate() {
                    let wave = ((i * (m + 3) + s * 11 + m * 7) % 19) as u64;
                    let gain = match m {
                        0 => {
                            if i % 41 < 12 {
                                17
                            } else {
                                5
                            }
                        }
                        1 => {
                            if i % 67 > 40 {
                                12
                            } else {
                                2
                            }
                        }
                        _ => {
                            if i % 29 < 5 {
                                18
                            } else {
                                1
                            }
                        }
                    };
                    if (i + s + m) % 7 == 0 {
                        continue;
                    }
                    let base = (wave + 1) * gain * 80;
                    usage.push(Usage {
                        model: model.id.clone(),
                        tokens: Tokens::known(
                            base,
                            base / 3 + (s * 11) as u64,
                            base * (1 + (i % 9) as u64),
                            base / 5,
                        ),
                    });
                }
                sessions.push(Session {
                    id,
                    day: start + i as i32,
                    seconds: Some(120 + ((i * 199 + s * 997) % 17900) as u64),
                    usage,
                });
            }
        }
        Dataset {
            start,
            end: start + 364,
            models,
            sessions,
        }
    })
}
