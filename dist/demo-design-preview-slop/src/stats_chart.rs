//! Shared chart painters for Stats and Settings previews.
use crate::{
    app::App,
    stats::{ChartStyle, StatsAction},
    stats_data::{self as data, Bucket, ModelTotal},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};
#[derive(Clone, Debug)]
pub struct ChartRows {
    pub lines: Vec<Line<'static>>,
    pub hits: Vec<(Rect, StatsAction)>,
}
pub fn ink(app: &App, index: usize, dim: bool) -> Style {
    let mut style = app.theme.base();
    if app.theme.color {
        style = style.fg(
            if app.theme.rgb && app.theme.mode == crate::theme::ThemeMode::Dark {
                let palette = [
                    (118, 185, 0),
                    (130, 184, 200),
                    (229, 180, 84),
                    (176, 147, 218),
                    (200, 153, 170),
                    (140, 150, 155),
                ];
                let (r, g, b) = if index == usize::MAX {
                    (140, 150, 155)
                } else {
                    palette.get(index).copied().unwrap_or_else(|| {
                        (
                            90 + (index.wrapping_mul(53) % 130) as u8,
                            100 + (index.wrapping_mul(71) % 120) as u8,
                            100 + (index.wrapping_mul(97) % 120) as u8,
                        )
                    })
                };
                if dim {
                    Color::Rgb(r / 2, g / 2, b / 2)
                } else {
                    Color::Rgb(r, g, b)
                }
            } else {
                [
                    Color::Green,
                    Color::Cyan,
                    Color::Yellow,
                    Color::Magenta,
                    Color::Blue,
                    Color::Gray,
                ][index % 6]
            },
        );
    }
    style
}
fn buffer_lines(b: &Buffer) -> Vec<Line<'static>> {
    (0..b.area.height)
        .map(|y| {
            Line::from(
                (0..b.area.width)
                    .map(|x| Span::styled(b[(x, y)].symbol().to_owned(), b[(x, y)].style()))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}
fn put(b: &mut Buffer, x: usize, y: usize, s: &str, style: Style) {
    if x < b.area.width as usize && y < b.area.height as usize {
        b.set_string(x as u16, y as u16, s, style);
    }
}
fn plot_line(
    b: &mut Buffer,
    area: Rect,
    values: &[Option<u64>],
    max: u64,
    identity: (usize, usize),
    app: &App,
    dim: bool,
) {
    let (model, color) = identity;
    let ascii = app.ascii || !app.theme.color;
    let sx = if ascii { 1 } else { 2 };
    let sy = if ascii { 1 } else { 4 };
    let width = usize::from(area.width) * sx;
    let height = usize::from(area.height) * sy;
    if width == 0 || height == 0 {
        return;
    }
    let points = values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            value.map(|v| {
                (
                    (i * (width - 1) / values.len().saturating_sub(1).max(1)) as i32,
                    (height
                        - 1
                        - ((u128::from(v) * (height - 1) as u128 / u128::from(max.max(1)))
                            as usize)
                            .min(height - 1)) as i32,
                )
            })
        })
        .collect::<Vec<_>>();
    let mut pixels = vec![false; width * height];
    for (i, point) in points.iter().enumerate() {
        let Some((mut x, mut y)) = *point else {
            continue;
        };
        let (endx, endy) = points.get(i + 1).copied().flatten().unwrap_or((x, y));
        let dx = (endx - x).abs();
        let sx = if x < endx { 1 } else { -1 };
        let dy = -(endy - y).abs();
        let sy = if y < endy { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            pixels[y as usize * width + x as usize] = true;
            if x == endx && y == endy {
                break;
            }
            let e = 2 * err;
            if e >= dy {
                err += dy;
                x += sx;
            }
            if e <= dx {
                err += dx;
                y += sy;
            }
        }
    }
    for cy in 0..area.height as usize {
        for cx in 0..area.width as usize {
            if ascii {
                if pixels[cy * width + cx] {
                    put(
                        b,
                        area.x as usize + cx,
                        area.y as usize + cy,
                        &char::from_digit(model as u32 + 1, 10)
                            .unwrap_or('*')
                            .to_string(),
                        ink(app, color, dim),
                    );
                }
            } else {
                let mut mask = 0;
                for (px, py, bit) in [
                    (0, 0, 0),
                    (0, 1, 1),
                    (0, 2, 2),
                    (1, 0, 3),
                    (1, 1, 4),
                    (1, 2, 5),
                    (0, 3, 6),
                    (1, 3, 7),
                ] {
                    if pixels[(cy * 4 + py) * width + cx * 2 + px] {
                        mask |= 1 << bit;
                    }
                }
                if mask != 0 {
                    let cell = &mut b[(area.x + cx as u16, area.y + cy as u16)];
                    let old = cell
                        .symbol()
                        .chars()
                        .next()
                        .filter(|c| ('\u{2800}'..='\u{28ff}').contains(c))
                        .map_or(0, |c| c as u32 - 0x2800);
                    cell.set_symbol(&char::from_u32(0x2800 | mask | old).unwrap().to_string())
                        .set_style(ink(app, color, dim));
                }
            }
        }
    }
}
fn total(bucket: &Bucket) -> Option<u64> {
    bucket
        .models
        .iter()
        .try_fold(0u64, |a, b| a.checked_add((*b)?))
}
/// A panel never drops a bucket. Narrow charts split at whole bucket boundaries.
pub fn panels(width: u16, count: usize) -> Vec<std::ops::Range<usize>> {
    let available = usize::from(width.saturating_sub(9)).max(1);
    let n = count.div_ceil(available).max(1);
    let size = count.div_ceil(n).max(1);
    (0..count)
        .step_by(size)
        .map(|s| s..(s + size).min(count))
        .collect()
}
pub fn timeline(
    app: &App,
    style: ChartStyle,
    width: u16,
    height: u16,
    buckets: &[Bucket],
    models: &[ModelTotal],
    highlight: Option<usize>,
) -> ChartRows {
    let mut lines = vec![];
    let mut hits = vec![];
    let peak = buckets
        .iter()
        .map(|b| {
            b.models
                .iter()
                .flatten()
                .try_fold(0u64, |sum, n| sum.checked_add(*n))
                .unwrap_or(u64::MAX)
        })
        .max()
        .unwrap_or(0)
        .max(1);
    for range in panels(width, buckets.len()) {
        let selected = &buckets[range.clone()];
        let plot = Rect::new(8, 0, width.saturating_sub(9), height);
        let mut b = Buffer::empty(Rect::new(0, 0, width, height + 2));
        b.set_style(b.area, app.theme.base());
        for (y, value) in [
            (0, peak),
            (height / 2, peak / 2),
            (height.saturating_sub(1), 0),
        ] {
            put(
                &mut b,
                0,
                y as usize,
                &format!("{:>6} ", data::number(value)),
                app.theme.muted(),
            );
        }
        for x in plot.x..plot.right() {
            put(
                &mut b,
                x as usize,
                height as usize,
                if app.ascii { "-" } else { "─" },
                app.theme.rule(),
            );
        }
        if style == ChartStyle::Stacked {
            for (i, bucket) in selected.iter().enumerate() {
                let x = plot.x as usize + i * plot.width as usize / selected.len();
                let next = plot.x as usize + (i + 1) * plot.width as usize / selected.len();
                let barwidth = next.saturating_sub(x + 1).clamp(1, 3);
                let units = if app.ascii { 1 } else { 2 };
                let rows = height as usize * units;
                let mut stack = 0u128;
                if total(bucket).is_none() {
                    put(
                        &mut b,
                        x,
                        height.saturating_sub(1) as usize,
                        "?",
                        app.theme.muted(),
                    );
                    continue;
                }
                for (m, tokens) in bucket.models.iter().enumerate() {
                    let low = stack * rows as u128 / u128::from(peak);
                    stack += u128::from(tokens.unwrap_or(0));
                    let high = stack * rows as u128 / u128::from(peak);
                    for unit in low as usize..(high as usize).min(rows) {
                        let y = height as usize - 1 - unit / units;
                        let glyph = if app.ascii || !app.theme.color {
                            char::from_digit(m as u32 + 1, 10)
                                .unwrap_or('*')
                                .to_string()
                        } else if unit % 2 == 0 {
                            "▄".into()
                        } else {
                            "█".into()
                        };
                        for dx in 0..barwidth {
                            put(
                                &mut b,
                                x + dx,
                                y,
                                &glyph,
                                ink(app, models[m].color, highlight.is_some_and(|h| h != m)),
                            );
                        }
                    }
                }
            }
        } else {
            let mut order = (0..models.len()).collect::<Vec<_>>();
            if let Some(m) = highlight {
                order.retain(|&i| i != m);
                order.push(m);
            }
            for m in order {
                let values = selected.iter().map(|b| b.models[m]).collect::<Vec<_>>();
                plot_line(
                    &mut b,
                    plot,
                    &values,
                    peak,
                    (m, models[m].color),
                    app,
                    highlight.is_some_and(|h| h != m),
                );
            }
        }
        let first = data::date(selected[0].start);
        let last = data::date(selected.last().unwrap().end);
        put(
            &mut b,
            plot.x as usize,
            (height + 1) as usize,
            &first,
            app.theme.muted(),
        );
        put(
            &mut b,
            plot.right().saturating_sub(last.len() as u16) as usize,
            (height + 1) as usize,
            &last,
            app.theme.muted(),
        );
        let start_y = lines.len() as u16;
        for (i, _) in selected.iter().enumerate() {
            let x = plot.x + i as u16 * plot.width / selected.len() as u16;
            let end = plot.x + (i + 1) as u16 * plot.width / selected.len() as u16;
            hits.push((
                Rect::new(x, start_y, end.saturating_sub(x).max(1), height),
                StatsAction::Inspect(range.start + i),
            ));
        }
        lines.extend(buffer_lines(&b));
        lines.push(Line::from(""));
    }
    ChartRows { lines, hits }
}
pub fn mini(
    app: &App,
    width: u16,
    height: u16,
    buckets: &[Bucket],
    model: usize,
    color: usize,
) -> Vec<Line<'static>> {
    let mut out = vec![];
    let values = buckets.iter().map(|b| b.models[model]).collect::<Vec<_>>();
    let peak = values.iter().flatten().copied().max().unwrap_or(0).max(1);
    for range in panels(width + 9, buckets.len()) {
        let mut b = Buffer::empty(Rect::new(0, 0, width, height));
        b.set_style(b.area, app.theme.base());
        let area = b.area;
        plot_line(
            &mut b,
            area,
            &values[range],
            peak,
            (model, color),
            app,
            false,
        );
        out.extend(buffer_lines(&b));
    }
    out
}
pub fn preview(app: &App, style: ChartStyle, width: u16) -> Vec<Line<'static>> {
    let d = data::demo();
    let s = d.summary(d.end - 6, d.end);
    let m = data::chart_models(&s);
    let buckets = d.buckets(d.end - 6, d.end, false, &m);
    if style == ChartStyle::Trends {
        mini(app, width, 2, &buckets, 0, m[0].color)
    } else {
        timeline(app, style, width, 2, &buckets, &m, None)
            .lines
            .into_iter()
            .take(2)
            .collect()
    }
}
