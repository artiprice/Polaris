//! Structured, local-only prompt settings. This module never invokes services.
use crate::composer::Composer;
use std::{collections::BTreeMap, ops::Range};
pub const SETTINGS: &[(&str, &str)] = &[
    ("$websearch", "Enable web search (session preview setting)"),
    ("$subagents", "Enable subagents (session preview setting)"),
    (
        "$subagents-model",
        "Models and effort ranges: =[model(low-xhigh)]",
    ),
    ("$rag", "Retrieval sources: =[source1, source2]"),
    ("$skills", "Session skills: =[skill1, skill2]"),
    (
        "$links",
        "Reference links: =[https://example.com, https://example.org]",
    ),
];
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingValue {
    Toggle(bool),
    Text(String),
    List(Vec<String>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentModel {
    pub model: String,
    pub min_effort: Option<String>,
    pub max_effort: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptSettings {
    pub websearch: Option<bool>,
    pub subagents: Option<bool>,
    pub subagent_models: Option<Vec<SubagentModel>>,
    pub rag: Option<Vec<String>>,
    pub links: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub extra: BTreeMap<String, SettingValue>,
}
impl PromptSettings {
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(v) = self.websearch {
            lines.push(format!("$websearch={}", if v { "on" } else { "off" }));
        }
        if let Some(v) = self.subagents {
            lines.push(format!("$subagents={}", if v { "on" } else { "off" }));
        }
        if let Some(models) = &self.subagent_models {
            let values = models
                .iter()
                .map(|m| match (&m.min_effort, &m.max_effort) {
                    (Some(low), Some(high)) => format!("{}({low}-{high})", m.model),
                    _ => m.model.clone(),
                })
                .collect::<Vec<_>>();
            lines.push(format!("$subagents-model=[{}]", values.join(", ")));
        }
        for (key, list) in [
            ("rag", &self.rag),
            ("links", &self.links),
            ("skills", &self.skills),
        ] {
            if let Some(values) = list {
                lines.push(format!("{}{}=[{}]", char::from(36), key, values.join(", ")));
            }
        }
        for (key, value) in &self.extra {
            let value = match value {
                SettingValue::Toggle(v) => {
                    if *v {
                        "on".into()
                    } else {
                        "off".into()
                    }
                }
                SettingValue::Text(s) => s.clone(),
                SettingValue::List(items) => format!("[{}]", items.join(", ")),
            };
            lines.push(format!("{}{}={value}", char::from(36), key));
        }
        lines
    }
    pub fn completion_detail(&self, name: &str, help: &str) -> String {
        let prefix = format!("{name}=");
        self.summary_lines()
            .iter()
            .find_map(|line| line.strip_prefix(&prefix))
            .map_or_else(|| help.to_owned(), |value| format!("{value} · {help}"))
    }
    pub fn apply_changes(&mut self, changes: &Self) {
        if let Some(value) = changes.websearch {
            self.websearch = Some(value);
        }
        if let Some(value) = changes.subagents {
            self.subagents = Some(value);
        }
        if let Some(value) = &changes.subagent_models {
            self.subagent_models = Some(value.clone());
        }
        if let Some(value) = &changes.rag {
            self.rag = Some(value.clone());
        }
        if let Some(value) = &changes.links {
            self.links = Some(value.clone());
        }
        if let Some(value) = &changes.skills {
            self.skills = Some(value.clone());
        }
        self.extra.extend(changes.extra.clone());
    }
}
#[derive(Default, Debug)]
pub struct ParsedSettings {
    pub settings: PromptSettings,
    pub ranges: Vec<Range<usize>>,
    pub errors: Vec<String>,
}
fn key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')
}
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_owned()
    } else {
        s.to_owned()
    }
}
fn list_items(s: &str) -> Result<Vec<String>, String> {
    let mut items = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut depth = 0usize;
    for (i, ch) in s.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Err("unmatched ')' in list".into());
                }
                depth -= 1;
            }
            '[' | ']' => return Err("nested lists are not supported".into()),
            ',' if depth == 0 => {
                let item = unquote(&s[start..i]);
                if item.is_empty() {
                    return Err("empty list item".into());
                }
                items.push(item);
                start = i + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() || depth != 0 {
        return Err("unclosed quote or effort range".into());
    }
    let last = unquote(&s[start..]);
    if !last.is_empty() {
        items.push(last);
    } else if !items.is_empty() {
        return Err("empty list item".into());
    }
    Ok(items)
}
fn model_item(item: &str) -> Result<SubagentModel, String> {
    let mut result = SubagentModel {
        model: item.trim().into(),
        min_effort: None,
        max_effort: None,
    };
    if let Some(open) = item.rfind('(') {
        let range = item[open + 1..]
            .strip_suffix(')')
            .ok_or("unclosed model effort")?
            .trim();
        result.model = unquote(&item[..open]);
        let range = range.replace("-to-", "-");
        let (low, high) = range.split_once('-').unwrap_or((&range, &range));
        let low = low.trim();
        let high = high.trim();
        let min = crate::app::EFFORTS
            .iter()
            .position(|e| *e == low)
            .ok_or("unknown minimum effort")?;
        let max = crate::app::EFFORTS
            .iter()
            .position(|e| *e == high)
            .ok_or("unknown maximum effort")?;
        if min > max {
            return Err("effort range must run from lower to higher effort".into());
        }
        result.min_effort = Some(low.into());
        result.max_effort = Some(high.into());
    }
    if result.model.is_empty() {
        return Err("model name is required".into());
    }
    Ok(result)
}
fn apply(settings: &mut PromptSettings, key: &str, value: SettingValue) -> Result<(), String> {
    match key {
        "websearch" | "subagents" => {
            let flag = match value {
                SettingValue::Toggle(v) => v,
                SettingValue::List(v) if v.len() == 1 => match v[0].to_ascii_lowercase().as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("expected on or off".into()),
                },
                _ => return Err("expected on or off".into()),
            };
            if key == "websearch" {
                settings.websearch = Some(flag);
            } else {
                settings.subagents = Some(flag);
            }
        }
        "subagents-model" | "rag" | "links" | "skills" => {
            let SettingValue::List(items) = value else {
                return Err("expected =[item1, item2]".into());
            };
            match key {
                "subagents-model" => {
                    settings.subagent_models = Some(
                        items
                            .iter()
                            .map(|s| model_item(s))
                            .collect::<Result<_, _>>()?,
                    )
                }
                "rag" => settings.rag = Some(items),
                "skills" => settings.skills = Some(items),
                _ => settings.links = Some(items),
            }
        }
        _ => {
            settings.extra.insert(key.into(), value);
        }
    }
    Ok(())
}
/// Read only typed source, outside code and attachment chips. Errors preserve the draft.
pub fn parse(draft: &Composer) -> ParsedSettings {
    let text = &draft.text;
    let mut out = ParsedSettings::default();
    let mut i = 0;
    let mut code = 0;
    while i < text.len() {
        if let Some(span) = draft.spans.iter().find(|s| s.range.contains(&i)) {
            i = span.range.end;
            continue;
        }
        let ch = text[i..].chars().next().unwrap();
        if ch == '`' {
            let n = text[i..].bytes().take_while(|c| *c == b'`').count();
            if code == 0 {
                code = n;
            } else if code == n {
                code = 0;
            }
            i += n;
            continue;
        }
        let boundary = i == 0
            || text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || "([{,;".contains(c));
        if code != 0 || ch != '$' || !boundary {
            i += ch.len_utf8();
            continue;
        }
        let start = i;
        i += 1;
        let key_start = i;
        if !text[i..].starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_') {
            continue;
        }
        while i < text.len() && key_char(text[i..].chars().next().unwrap()) {
            i += 1;
        }
        if i == key_start {
            continue;
        }
        let key = &text[key_start..i];
        let name_end = i;
        let mut next = i;
        while next < text.len() && matches!(text.as_bytes()[next], b' ' | b'\t') {
            next += 1;
        }
        let value = if text.as_bytes().get(next) == Some(&b'=') {
            i = next + 1;
            while i < text.len() && matches!(text.as_bytes()[i], b' ' | b'\t') {
                i += 1;
            }
            let value_start = i;
            if text.as_bytes().get(i) == Some(&b'[') {
                i += 1;
                let inner = i;
                let mut quote = None;
                while i < text.len() {
                    let c = text[i..].chars().next().unwrap();
                    if let Some(q) = quote {
                        if c == q {
                            quote = None;
                        }
                    } else if matches!(c, '"' | '\'') {
                        quote = Some(c);
                    } else if c == ']' {
                        break;
                    }
                    i += c.len_utf8();
                }
                if i == text.len() {
                    Err("unclosed list: add ']'".into())
                } else {
                    let result = list_items(&text[inner..i]).map(SettingValue::List);
                    i += 1;
                    if i < text.len() && !text[i..].starts_with(char::is_whitespace) {
                        while i < text.len() && !text[i..].starts_with(char::is_whitespace) {
                            i += text[i..].chars().next().unwrap().len_utf8();
                        }
                        Err("separate settings with whitespace".into())
                    } else {
                        result
                    }
                }
            } else {
                let mut quote = None;
                if matches!(text.as_bytes().get(i), Some(b'\'' | b'"')) {
                    quote = text[i..].chars().next();
                    i += 1;
                }
                if let Some(q) = quote {
                    while i < text.len() && !text[i..].starts_with(q) {
                        i += text[i..].chars().next().unwrap().len_utf8();
                    }
                    if i < text.len() {
                        i += q.len_utf8();
                    } else {
                        out.errors.push(format!("{}{}: unclosed quote", '$', key));
                    }
                } else {
                    while i < text.len() && !text[i..].starts_with(char::is_whitespace) {
                        i += text[i..].chars().next().unwrap().len_utf8();
                    }
                }
                let raw = unquote(&text[value_start..i]);
                match raw.to_ascii_lowercase().as_str() {
                    "on" => Ok(SettingValue::Toggle(true)),
                    "off" => Ok(SettingValue::Toggle(false)),
                    "" => Err("value is required after '='".into()),
                    _ => Ok(SettingValue::Text(raw)),
                }
            }
        } else {
            i = name_end;
            Ok(SettingValue::Toggle(true))
        };
        out.ranges.push(start..i);
        if let Err(error) = value.and_then(|v| apply(&mut out.settings, key, v)) {
            out.errors.push(format!("{}{}: {error}", '$', key));
        }
    }
    out
}
