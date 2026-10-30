//! Compact questionnaire overlay projection.

use polaris_ui_model::questionnaire::{
    ChoiceAnswer, QuestionAnswer, QuestionKind, QuestionnaireFocus,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::{ColorMode, HitRegion, HitTarget, QuestionnaireOverlay, RenderOptions};

const TEXT: Color = Color::Rgb(232, 232, 232);
const MUTED: Color = Color::Rgb(133, 133, 133);
const ACCENT: Color = Color::Rgb(118, 185, 0);
const HOVER: Color = Color::Rgb(35, 56, 0);
const PANEL: Color = Color::Rgb(0, 0, 0);
const RULE: Color = Color::Rgb(52, 52, 52);
const MAX_EDITOR_ROWS: usize = 4;

pub(super) fn overlay_rect(
    composer: Rect,
    viewport: Rect,
    overlay: QuestionnaireOverlay<'_>,
) -> Option<Rect> {
    let cap = viewport
        .height
        .saturating_sub(8)
        .min(9_u16.max(viewport.height / 2));
    if cap == 0 || composer.width == 0 {
        return None;
    }
    let content_rows = if overlay.review {
        overlay
            .state
            .spec()
            .questions
            .len()
            .saturating_mul(2)
            .saturating_add(4)
    } else {
        let question_rows = overlay
            .state
            .current_question()
            .map_or(1, |question| match &question.kind {
                QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } => {
                    options.len().saturating_add(1)
                }
                QuestionKind::FreeText => answer_rows(overlay, composer.width.saturating_sub(2)),
            });
        question_rows
            .saturating_add(usize::from(note_rows(
                overlay,
                composer.width.saturating_sub(2),
            )))
            .saturating_add(6)
    };
    let desired = u16::try_from(content_rows).unwrap_or(u16::MAX).max(9);
    let height = desired.min(cap).min(viewport.height.saturating_sub(1));
    let bottom = composer.bottom().min(viewport.bottom());
    let y = bottom.saturating_sub(height).max(viewport.y);
    let actual_height = bottom.saturating_sub(y);
    (actual_height > 0).then_some(Rect::new(composer.x, y, composer.width, actual_height))
}

pub(super) fn append_hits(
    hits: &mut Vec<HitRegion>,
    area: Rect,
    overlay: QuestionnaireOverlay<'_>,
    _options: RenderOptions,
) -> Option<super::QuestionnaireEditorGeometry> {
    if area.width < 4 || area.height < 5 {
        return None;
    }
    let inner_x = area.x.saturating_add(1);
    let inner_width = area.width.saturating_sub(2);
    let tab_y = area.y.saturating_add(if overlay.review { 1 } else { 2 });
    let mut x = inner_x.saturating_add(if overlay.review { 8 } else { 0 });
    for index in 0..overlay.state.spec().questions.len() {
        let width = 4_u16.min(area.right().saturating_sub(1).saturating_sub(x));
        if width == 0 {
            break;
        }
        hits.push(HitRegion {
            rect: Rect::new(x, tab_y, width, 1),
            target: HitTarget::QuestionTab(index),
        });
        x = x.saturating_add(width);
    }
    let submit_width = 10_u16.min(area.right().saturating_sub(1).saturating_sub(x));
    if submit_width > 0 {
        hits.push(HitRegion {
            rect: Rect::new(x, tab_y, submit_width, 1),
            target: HitTarget::QuestionSubmit,
        });
    }

    if overlay.review {
        return None;
    }
    let question = overlay.state.current_question()?;
    let choice_top = area.y.saturating_add(4);
    let notes_lines = effective_note_rows(area, overlay, inner_width);
    let notes_y = area.bottom().saturating_sub(2).saturating_sub(notes_lines);
    let choice_rows = notes_y.saturating_sub(choice_top);
    let choice_count = match &question.kind {
        QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } => {
            options.len().saturating_add(1)
        }
        QuestionKind::FreeText => 0,
    };
    let visible_named =
        usize::from(choice_rows.saturating_sub(1)).min(choice_count.saturating_sub(1));
    let first = choice_first(
        overlay.state.choice_cursor(),
        choice_count.saturating_sub(1),
        visible_named,
    );
    for offset in 0..visible_named {
        let y = choice_top.saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        hits.push(HitRegion {
            rect: Rect::new(inner_x, y, inner_width, 1),
            target: HitTarget::QuestionChoice(first.saturating_add(offset)),
        });
    }
    if choice_count > 0 && choice_rows > 0 {
        hits.push(HitRegion {
            rect: Rect::new(
                inner_x,
                choice_top.saturating_add(choice_rows.saturating_sub(1)),
                inner_width,
                1,
            ),
            target: HitTarget::QuestionChoice(choice_count.saturating_sub(1)),
        });
    }
    hits.push(HitRegion {
        rect: Rect::new(inner_x, notes_y, inner_width, notes_lines),
        target: HitTarget::QuestionNotes,
    });
    let surface = editor_surface(
        overlay,
        inner_x,
        choice_top,
        choice_rows,
        inner_width,
        notes_y,
        notes_lines,
    );
    if let Some(editor) = surface.as_ref()
        && overlay.state.focus() != QuestionnaireFocus::Notes
    {
        hits.push(HitRegion {
            rect: editor.rect,
            target: HitTarget::QuestionAnswer,
        });
    }
    surface
}

pub(super) fn draw(
    buffer: &mut Buffer,
    area: Rect,
    overlay: QuestionnaireOverlay<'_>,
    options: RenderOptions,
) -> Option<super::CaretPoint> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let bg = match options.colors {
        ColorMode::TrueColor => PANEL,
        ColorMode::Ansi => Color::Indexed(0),
        ColorMode::Terminal | ColorMode::NoColor => Color::Reset,
    };
    buffer.set_style(area, Style::default().bg(bg));
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_style(Style::default().bg(bg));
            }
        }
    }
    border(buffer, area, options);
    if area.height < 5 || area.width < 4 {
        return None;
    }
    let x = area.x.saturating_add(1);
    let width = area.width.saturating_sub(2);
    let tab_y = area.y.saturating_add(if overlay.review { 1 } else { 2 });
    let hint_y = area.bottom().saturating_sub(2);
    let title = if overlay.review {
        " Review".to_owned()
    } else {
        " Questions".to_owned()
    };
    write(
        buffer,
        x,
        area.y.saturating_add(1),
        &title,
        width,
        style(options, TEXT, true),
    );
    let tabs_x = if overlay.review {
        x.saturating_add(8)
    } else {
        x
    };
    let tabs_width = width.saturating_sub(tabs_x.saturating_sub(x));
    draw_tabs(buffer, tabs_x, tab_y, tabs_width, overlay, options);
    let mut caret = None;
    if overlay.review {
        draw_review(
            buffer,
            x,
            tab_y.saturating_add(1),
            hint_y,
            width,
            overlay,
            options,
        );
    } else if let Some(question) = overlay.state.current_question() {
        write(
            buffer,
            x,
            area.y.saturating_add(3),
            &question.prompt,
            width,
            style(options, TEXT, false),
        );
        let choice_top = area.y.saturating_add(4);
        let notes_lines = effective_note_rows(area, overlay, width);
        let notes_y = area.bottom().saturating_sub(2).saturating_sub(notes_lines);
        draw_choices(buffer, x, choice_top, notes_y, width, overlay, options);
        if matches!(question.kind, QuestionKind::FreeText) {
            let answer_height = notes_y.saturating_sub(choice_top);
            draw_answer_editor(
                buffer,
                x,
                choice_top,
                width,
                answer_height,
                overlay,
                options,
            );
        }
        draw_notes_editor(buffer, x, notes_y, width, notes_lines, overlay, options);
        if overlay.state.focus() == QuestionnaireFocus::Notes
            || matches!(question.kind, QuestionKind::FreeText)
        {
            caret = editor_caret(
                overlay,
                x,
                if overlay.state.focus() == QuestionnaireFocus::Notes {
                    notes_y
                } else {
                    choice_top
                },
                width,
                if overlay.state.focus() == QuestionnaireFocus::Notes {
                    notes_lines
                } else {
                    notes_y.saturating_sub(choice_top)
                },
            );
        }
    }
    write(
        buffer,
        x,
        hint_y,
        "Tab notes | Enter review/submit | arrows | Esc interrupt",
        width,
        style(options, MUTED, false),
    );
    caret
}

fn draw_tabs(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    overlay: QuestionnaireOverlay<'_>,
    options: RenderOptions,
) {
    let active = (!overlay.review).then_some(overlay.state.question_index());
    let mut col = x;
    for (index, _) in overlay.state.spec().questions.iter().enumerate() {
        let selected = active == Some(index);
        let label = format!("{}{} ", if selected { "[" } else { " " }, index + 1);
        let close = if selected { "]" } else { " " };
        let text = format!("{label}{close}");
        let tab_style = if selected {
            style(options, ACCENT, true)
        } else {
            style(options, MUTED, false)
        };
        let used = u16::try_from(text.len())
            .unwrap_or(u16::MAX)
            .min(x.saturating_add(width).saturating_sub(col));
        if used == 0 {
            break;
        }
        write(buffer, col, y, &text, used, tab_style);
        col = col.saturating_add(used);
    }
    let (submit, submit_style) = if overlay.review {
        ("[ Submit ]", style(options, ACCENT, true))
    } else {
        (" Review ", style(options, TEXT, true))
    };
    if col < x.saturating_add(width) {
        write(
            buffer,
            col,
            y,
            submit,
            x.saturating_add(width).saturating_sub(col),
            submit_style,
        );
    }
}

fn note_text(overlay: QuestionnaireOverlay<'_>) -> String {
    overlay
        .state
        .draft_answers()
        .get(overlay.state.question_index())
        .map_or_else(String::new, |entry| entry.notes.clone())
}

fn note_rows(overlay: QuestionnaireOverlay<'_>, width: u16) -> u16 {
    let geometry =
        super::EditorGeometry::new(&note_text(overlay), &[], width.saturating_sub(8).max(1));
    if overlay.state.focus() == QuestionnaireFocus::Notes {
        u16::try_from(geometry.rows().len().min(4))
            .unwrap_or(4)
            .max(1)
    } else {
        1
    }
}

fn effective_note_rows(area: Rect, overlay: QuestionnaireOverlay<'_>, width: u16) -> u16 {
    let hint_y = area.bottom().saturating_sub(2);
    let choice_top = area.y.saturating_add(4);
    let max_notes = hint_y.saturating_sub(choice_top).saturating_sub(2).max(1);
    note_rows(overlay, width).min(max_notes)
}

fn answer_rows(overlay: QuestionnaireOverlay<'_>, width: u16) -> usize {
    let Some(question) = overlay.state.current_question() else {
        return 1;
    };
    if !matches!(question.kind, QuestionKind::FreeText) {
        return 1;
    }
    let geometry = super::EditorGeometry::new(
        overlay.state.answer_text(),
        &[],
        width.saturating_sub(8).max(1),
    );
    geometry.rows().len().clamp(1, MAX_EDITOR_ROWS)
}

fn editor_surface(
    overlay: QuestionnaireOverlay<'_>,
    x: u16,
    answer_y: u16,
    answer_height: u16,
    width: u16,
    notes_y: u16,
    notes_height: u16,
) -> Option<super::QuestionnaireEditorGeometry> {
    let (text, cursor, rect) = if overlay.state.focus() == QuestionnaireFocus::Notes {
        let editor = overlay.state.active_editor()?;
        (
            editor.text(),
            editor.cursor(),
            Rect::new(
                x.saturating_add(8),
                notes_y,
                width.saturating_sub(8),
                notes_height,
            ),
        )
    } else {
        let question = overlay.state.current_question()?;
        if !matches!(question.kind, QuestionKind::FreeText) {
            return None;
        }
        let editor = overlay.state.active_editor()?;
        (
            editor.text(),
            editor.cursor(),
            Rect::new(
                x.saturating_add(8),
                answer_y,
                width.saturating_sub(8),
                answer_height,
            ),
        )
    };
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let geometry = super::EditorGeometry::new(text, &[], rect.width);
    let cursor = geometry.caret_position(cursor);
    let visible_rows = geometry.visible_rows(usize::from(rect.height), cursor.row);
    Some(super::QuestionnaireEditorGeometry {
        rect,
        geometry,
        visible_rows,
    })
}

fn editor_caret(
    overlay: QuestionnaireOverlay<'_>,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
) -> Option<super::CaretPoint> {
    let surface = editor_surface(overlay, x, y, height, width, y, height)?;
    let editor = overlay.state.active_editor()?;
    let cursor = surface.geometry.caret_position(editor.cursor());
    if !surface.visible_rows.contains(&cursor.row) {
        return None;
    }
    let row = cursor.row.saturating_sub(surface.visible_rows.start);
    Some(super::CaretPoint {
        x: surface
            .rect
            .x
            .saturating_add(u16::try_from(cursor.column).unwrap_or(u16::MAX)),
        y: surface
            .rect
            .y
            .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
    })
}

fn draw_notes_editor(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    overlay: QuestionnaireOverlay<'_>,
    options: RenderOptions,
) {
    let notes = note_text(overlay);
    let editable = overlay.state.focus() == QuestionnaireFocus::Notes;
    let text = if editable {
        overlay
            .state
            .active_editor()
            .map_or(notes.as_str(), |editor| editor.text())
    } else {
        notes.as_str()
    };
    let label = " Notes: ";
    write(buffer, x, y, label, width, style(options, MUTED, false));
    let field_x = x.saturating_add(8);
    let field_width = width.saturating_sub(8);
    if field_width == 0 || height == 0 {
        return;
    }
    let (cursor, selection) = if editable {
        overlay
            .state
            .active_editor()
            .map_or((0, None), |editor| (editor.cursor(), editor.selection()))
    } else {
        (0, None)
    };
    draw_editor_text(
        buffer,
        text,
        cursor,
        selection.as_ref(),
        field_x,
        y,
        field_width,
        height,
        options,
    );
}

fn draw_answer_editor(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    overlay: QuestionnaireOverlay<'_>,
    options: RenderOptions,
) {
    write(
        buffer,
        x,
        y,
        " Answer: ",
        width,
        style(options, MUTED, false),
    );
    let field_x = x.saturating_add(8);
    let field_width = width.saturating_sub(8);
    if field_width == 0 || height == 0 {
        return;
    }
    let text = overlay.state.answer_text();
    let editable = overlay.state.focus() == QuestionnaireFocus::Choices;
    let (cursor, selection) = if editable {
        overlay
            .state
            .active_editor()
            .map_or((0, None), |editor| (editor.cursor(), editor.selection()))
    } else {
        (0, None)
    };
    draw_editor_text(
        buffer,
        text,
        cursor,
        selection.as_ref(),
        field_x,
        y,
        field_width,
        height,
        options,
    );
}

fn draw_editor_text(
    buffer: &mut Buffer,
    text: &str,
    cursor: usize,
    selection: Option<&std::ops::Range<usize>>,
    field_x: u16,
    y: u16,
    field_width: u16,
    height: u16,
    options: RenderOptions,
) {
    if field_width == 0 || height == 0 {
        return;
    }
    let geometry = super::EditorGeometry::new(text, &[], field_width);
    let caret = geometry.caret_position(cursor);
    let visible = geometry.visible_rows(usize::from(height), caret.row);
    for (display_row, row_index) in visible.clone().enumerate() {
        let Some(row) = geometry.rows().get(row_index) else {
            continue;
        };
        let screen_y = y.saturating_add(u16::try_from(display_row).unwrap_or(u16::MAX));
        write(
            buffer,
            field_x,
            screen_y,
            &row.display_text,
            field_width,
            style(options, TEXT, false),
        );
        if let Some(selected) = selection {
            for span in &row.spans {
                if span.source.start < selected.end && selected.start < span.source.end {
                    super::shade_selection(
                        buffer,
                        Rect::new(
                            field_x.saturating_add(
                                u16::try_from(span.cells.start).unwrap_or(u16::MAX),
                            ),
                            screen_y,
                            u16::try_from(span.cells.end.saturating_sub(span.cells.start))
                                .unwrap_or(u16::MAX),
                            1,
                        ),
                        options,
                    );
                }
            }
        }
    }
}

fn draw_choices(
    buffer: &mut Buffer,
    x: u16,
    top: u16,
    notes_y: u16,
    width: u16,
    overlay: QuestionnaireOverlay<'_>,
    options: RenderOptions,
) {
    let Some(question) = overlay.state.current_question() else {
        return;
    };
    let options_list = match &question.kind {
        QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } => options,
        QuestionKind::FreeText => return,
    };
    let rows = usize::from(notes_y.saturating_sub(top));
    if rows == 0 {
        return;
    }
    let selected = overlay.state.selected_named_options();
    let total = options_list.len().saturating_add(1);
    let cursor = overlay.state.choice_cursor().min(total.saturating_sub(1));
    let named_slots = rows.saturating_sub(1).min(options_list.len());
    let first = choice_first(cursor, options_list.len(), named_slots);
    for offset in 0..named_slots {
        let index = first.saturating_add(offset);
        let Some(option) = options_list.get(index) else {
            continue;
        };
        let y = top.saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        let chosen = selected.iter().any(|id| id == &option.id);
        let label = format!(" {} {}", if chosen { "✓" } else { "○" }, option.label);
        if cursor == index
            && overlay.state.focus() == QuestionnaireFocus::Choices
            && options.colors == ColorMode::TrueColor
        {
            buffer.set_style(Rect::new(x, y, width, 1), Style::default().bg(HOVER));
        }
        write(
            buffer,
            x,
            y,
            &label,
            width,
            style(options, if chosen { ACCENT } else { TEXT }, chosen),
        );
    }
    if rows > 0 {
        let y = top.saturating_add(u16::try_from(rows.saturating_sub(1)).unwrap_or(u16::MAX));
        let chosen = overlay.state.none_of_the_above_selected();
        let label = format!(" {} None of the above", if chosen { "✓" } else { "○" });
        if cursor == total.saturating_sub(1)
            && overlay.state.focus() == QuestionnaireFocus::Choices
            && options.colors == ColorMode::TrueColor
        {
            buffer.set_style(Rect::new(x, y, width, 1), Style::default().bg(HOVER));
        }
        write(
            buffer,
            x,
            y,
            &label,
            width,
            style(options, if chosen { ACCENT } else { TEXT }, chosen),
        );
    }
}

fn draw_review(
    buffer: &mut Buffer,
    x: u16,
    top: u16,
    end: u16,
    width: u16,
    overlay: QuestionnaireOverlay<'_>,
    options: RenderOptions,
) {
    let lines = review_lines(overlay, width);
    let visible = usize::from(end.saturating_sub(top));
    let first = overlay.scroll.min(lines.len().saturating_sub(visible));
    for (offset, line) in lines.iter().skip(first).take(visible).enumerate() {
        let y = top.saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        if y >= end {
            break;
        }
        write(buffer, x, y, line, width, style(options, TEXT, false));
    }
}

fn review_lines(overlay: QuestionnaireOverlay<'_>, width: u16) -> Vec<String> {
    let answers = overlay.state.draft_answers();
    let questions = &overlay.state.spec().questions;
    let mut lines = Vec::new();
    for (index, entry) in answers.iter().enumerate() {
        let question = questions.get(index);
        let answer = match &entry.answer {
            QuestionAnswer::Unanswered => "Unanswered".to_owned(),
            QuestionAnswer::SingleChoice(ChoiceAnswer::Option(id)) => option_label(question, id),
            QuestionAnswer::SingleChoice(ChoiceAnswer::NoneOfTheAbove) => {
                "None of the above".to_owned()
            }
            QuestionAnswer::MultiChoice(answer) => match answer {
                polaris_ui_model::questionnaire::MultiChoiceAnswer::Options(ids) => ids
                    .iter()
                    .map(|id| option_label(question, id))
                    .collect::<Vec<_>>()
                    .join(", "),
                polaris_ui_model::questionnaire::MultiChoiceAnswer::NoneOfTheAbove => {
                    "None of the above".to_owned()
                }
            },
            QuestionAnswer::FreeText(text) => text.clone(),
        };
        let prompt = question.map_or(entry.question_id.as_str(), |q| q.prompt.as_str());
        append_wrapped(
            &mut lines,
            &format!("{}: {} — {}", index.saturating_add(1), prompt, answer),
            width,
        );
        if !entry.notes.is_empty() {
            append_wrapped(&mut lines, &format!("   Notes: {}", entry.notes), width);
        }
    }
    lines
}

fn append_wrapped(lines: &mut Vec<String>, text: &str, width: u16) {
    let wrapped = crate::text::wrap_display_text(text, width.max(1), 0);
    lines.extend(wrapped.into_iter().map(|line| line.text));
}

fn option_label(
    question: Option<&polaris_ui_model::questionnaire::QuestionSpec>,
    id: &str,
) -> String {
    question
        .and_then(|q| match &q.kind {
            QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } => {
                options.iter().find(|o| o.id == id).map(|o| o.label.clone())
            }
            QuestionKind::FreeText => None,
        })
        .unwrap_or_else(|| id.to_owned())
}

fn choice_first(cursor: usize, named_count: usize, visible: usize) -> usize {
    if visible == 0 || named_count <= visible {
        0
    } else {
        cursor
            .saturating_sub(visible.saturating_sub(1))
            .min(named_count.saturating_sub(visible))
    }
}

fn border(buffer: &mut Buffer, area: Rect, options: RenderOptions) {
    let (tl, tr, bl, br, h, v) = if options.ascii {
        ('+', '+', '+', '+', '-', '|')
    } else {
        ('┌', '┐', '└', '┘', '─', '│')
    };
    let style = style(options, RULE, false);
    if area.width < 2 || area.height < 2 {
        return;
    }
    for x in area.x..area.right() {
        let ch = if x == area.x {
            tl
        } else if x == area.right().saturating_sub(1) {
            tr
        } else {
            h
        };
        if let Some(c) = buffer.cell_mut((x, area.y)) {
            c.set_char(ch);
            c.set_style(style);
        }
        let y = area.bottom().saturating_sub(1);
        let ch = if x == area.x {
            bl
        } else if x == area.right().saturating_sub(1) {
            br
        } else {
            h
        };
        if let Some(c) = buffer.cell_mut((x, y)) {
            c.set_char(ch);
            c.set_style(style);
        }
    }
    for y in area.y.saturating_add(1)..area.bottom().saturating_sub(1) {
        for (x, ch) in [(area.x, v), (area.right().saturating_sub(1), v)] {
            if let Some(c) = buffer.cell_mut((x, y)) {
                c.set_char(ch);
                c.set_style(style);
            }
        }
    }
}

fn write(buffer: &mut Buffer, x: u16, y: u16, text: &str, width: u16, style: Style) {
    if width == 0 {
        return;
    }
    buffer.set_stringn(
        x,
        y,
        crate::text::fit_display_text(text, width),
        usize::from(width),
        style,
    );
}

fn style(options: RenderOptions, color: Color, bold: bool) -> Style {
    let fg = match (options.colors, color) {
        (ColorMode::TrueColor, c) => c,
        (ColorMode::Ansi, Color::Rgb(118, 185, 0)) => Color::Green,
        (ColorMode::Ansi, Color::Rgb(133, 133, 133)) => Color::DarkGray,
        (ColorMode::Ansi, _) => Color::White,
        (ColorMode::Terminal, Color::Rgb(118, 185, 0)) => Color::LightGreen,
        (ColorMode::Terminal, Color::Rgb(133, 133, 133)) => Color::Gray,
        (ColorMode::Terminal, _) => Color::White,
        (ColorMode::NoColor, _) => Color::Reset,
    };
    let base = Style::default().fg(fg);
    if bold {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    }
}
