//! diff row measurement, selection and floating comment anchors

use blit::{Axis, Constraints, Input, Layout, LayoutCx, Platform, Point, PointerButton, Sense, Sides, Size, Sizing, WidgetId};
use blit_desktop::atom::Rectangle;
use blit_desktop::layout::{flex, single, Align};
use blit_desktop::widget::scroll;
use blit_desktop::{BoundsClip, Ui};

use super::review::{self, Comment, HeaderAction, Review, Thread};
use super::{
    lines,
    theme::{self, sz},
    widgets,
};
use crate::diff::Kind as Change;
use crate::text::Anchor;

#[derive(Default)]
pub struct State {
    pub list: scroll::MeasuredState,
    pub rows_dirty: bool,
    rows: Vec<Row>,
    label: String,
    follow_scroll: bool,
    split: bool,
}

struct Row {
    file: usize,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Header,
    Separator { heading: Option<usize>, context: Option<usize> },
    Code { left: Option<LineRef>, right: Option<LineRef> },
    Binary,
    Gap,
    Empty,
}

pub fn build(mut ui: Ui<'_>, review: &mut Review, reveal: Option<usize>) -> Option<WidgetId> {
    let Review { files, entries, list: state, drag, thread, split, selected: selected_file, reveal_comment, .. } = review;
    let Some(Ok(files)) = files else { return None };
    let viewport = ui.geometry(state.list.id());
    let anchor = (state.split != *split)
        .then(|| {
            state.rows.iter().find_map(|row| {
                if !matches!(row.kind, Kind::Header | Kind::Code { .. }) {
                    return None;
                }
                let area = ui.geometry(row.id(state.split))?;
                let viewport = viewport?;
                (area.y + area.height > viewport.y && area.y < viewport.y + viewport.height).then_some((row.file, row.kind))
            })
        })
        .flatten();
    state.split = *split;
    if state.rows_dirty {
        state.rows_dirty = false;
        state.rows.clear();
        for (index, (file, &show)) in files.iter().zip(&entries.shown).enumerate() {
            if !show {
                continue;
            }
            state.rows.push(Row { file: index, kind: Kind::Header });
            if !file.collapsed {
                if file.diff.binary {
                    state.rows.push(Row { file: index, kind: Kind::Binary });
                }
                let mut flat = 0;
                for hunk in 0..=file.diff.hunks.len() {
                    let context = &file.context[hunk];
                    let length = context.total.unwrap_or(0);
                    for line in 0..context.before {
                        let line = Some(LineRef::Source { gap: hunk, line });
                        state.rows.push(Row { file: index, kind: Kind::Code { left: line, right: line } });
                    }
                    let controls = file.diff.source.is_some() && context.total != Some(0);
                    if controls {
                        let heading = (context.after == 0 && hunk < file.diff.hunks.len()).then_some(hunk);
                        state.rows.push(Row { file: index, kind: Kind::Separator { context: Some(hunk), heading } });
                    }
                    for line in length - context.after..length {
                        let line = Some(LineRef::Source { gap: hunk, line });
                        state.rows.push(Row { file: index, kind: Kind::Code { left: line, right: line } });
                    }
                    let Some(value) = file.diff.hunks.get(hunk) else { continue };
                    if (!controls || context.after > 0) && (hunk == 0 || length == 0 || context.before + context.after < length) {
                        state.rows.push(Row { file: index, kind: Kind::Separator { heading: Some(hunk), context: None } });
                    }
                    let mut line = 0;
                    while line < value.lines.len() {
                        let start = line;
                        let (left, right) = if *split && value.lines[line].kind != Change::Context {
                            while line < value.lines.len() && value.lines[line].kind == Change::Del {
                                line += 1;
                            }
                            let added = line;
                            while line < value.lines.len() && value.lines[line].kind == Change::Add {
                                line += 1;
                            }
                            (start..added, added..line)
                        } else {
                            line += 1;
                            (start..line, start..line)
                        };
                        for offset in 0..left.len().max(right.len()) {
                            let l = left.start + offset;
                            let r = right.start + offset;
                            let left = (l < left.end).then_some(LineRef::Diff { hunk, line: l, flat: flat + l });
                            let right = (*split && r < right.end).then_some(LineRef::Diff { hunk, line: r, flat: flat + r });
                            state.rows.push(Row { file: index, kind: Kind::Code { left, right } });
                        }
                    }
                    flat += value.lines.len();
                }
            }
            state.rows.push(Row { file: index, kind: Kind::Gap });
        }
        state.rows.pop();
        if state.rows.is_empty() {
            state.rows.push(Row { file: 0, kind: Kind::Empty });
        }
        state.list.mark_dirty();
    }
    let comment_target = reveal_comment.take();
    if let Some((file, anchor)) = comment_target {
        if let Some(row) = state.rows.iter().find(|row| {
            row.file == file
                && match anchor {
                    Anchor::File => matches!(row.kind, Kind::Header),
                    Anchor::Lines { end, .. } => matches!(row.kind, Kind::Code { left, right }
                if [left, right].into_iter().flatten().any(|line| matches!(line, LineRef::Diff { flat, .. } if flat == end))),
                }
        }) {
            state.list.scroll_to(row.id(*split));
        }
    } else if let Some(file) = reveal {
        state.list.scroll_to(Row { file, kind: Kind::Header }.id(*split));
    } else if let Some((file, kind)) = anchor {
        if let Some(row) = state.rows.iter().find(|row| {
            row.file == file
                && match kind {
                    Kind::Code { left, right } => {
                        let line = right.or(left);
                        matches!(row.kind, Kind::Code { left, right } if left == line || right == line)
                    }
                    _ => row.kind == kind,
                }
        }) {
            state.list.scroll_to(row.id(*split));
        }
    }

    let released = drag.is_some() && matches!(ui.input(), Input::PointerUp { button: PointerButton::Primary, .. });
    let cancelled = matches!(ui.input(), Input::Key(key) if key.pressed && key.key == blit::Key::Escape);
    if drag.is_some() {
        if released || cancelled || matches!(ui.input(), Input::PointerMove { .. }) {
            ui.request_frame();
        }
        if cancelled {
            *drag = None;
        }
    }
    let selected = *drag;
    let mut started = None;
    let mut header_action = None;
    let mut context_action = None;
    let mut open_anchor = None;
    if let Some(viewport) = viewport {
        match *ui.input() {
            Input::Scroll { position, .. } if viewport.contains(position) => state.follow_scroll = true,
            Input::PointerDown { position, .. }
                if viewport.contains(position) && position.x >= viewport.x + viewport.width - sz::SCROLLBAR =>
            {
                state.follow_scroll = true;
            }
            _ => {}
        }
    }
    if reveal.is_some() || comment_target.is_some() {
        state.follow_scroll = false;
    }
    let follow_scroll = state.follow_scroll;
    let mut visible_file = None;
    let response = ui.build(
        widgets::VirtualList::new(&mut state.list, BoundsClip, &state.rows)
            .key(|row| row.id(*split))
            .edge_scroll(selected.is_some() && !released)
            .build(|ui: Ui<'_>, row| {
                let mut ui = ui.widget_id(row.id(*split));
                if follow_scroll && visible_file.is_none() && matches!(row.kind, Kind::Header | Kind::Code { .. }) {
                    if let (Some(viewport), Some(area)) = (viewport, ui.geometry(row.id(*split))) {
                        if area.y + area.height > viewport.y && area.y < viewport.y + viewport.height {
                            visible_file = Some(row.file);
                            if *selected_file != row.file {
                                ui.request_frame();
                            }
                        }
                    }
                }
                if row.kind == Kind::Empty {
                    ui.insert(widgets::text(
                        if files.is_empty() { "No changes in this scope." } else { "No files match the filter." },
                        theme::interface(sz::TEXT_BODY),
                        theme::MUTED,
                    ));
                    return;
                }
                if row.kind == Kind::Gap {
                    ui.layout(single::layout().padding(Sides::y(sz::MD)));
                    return;
                }
                let file = &files[row.file];
                match row.kind {
                    Kind::Separator { heading, context } => {
                        use blit_desktop::text::{HorizontalAlign, TextOptions, VerticalAlign};
                        use ContextAction::*;
                        let mut bar =
                            ui.layout(flex::row().padding(Sides::y(if context.is_none() { sz::XXS } else { 0.0 })).align(Align::Center));
                        bar.insert(Rectangle::new().background(theme::HUNK));
                        let mut controls = bar.child(flex::item().width(Sizing::fixed(sz::GUTTER))).layout(flex::row());
                        if let Some(gap) = context {
                            let context = &file.context[gap];
                            let remaining = context.total.map(|count| count - context.before - context.after);
                            let small = remaining.is_some_and(|count| count <= 20);
                            let (symbol, expand) = if gap == 0 {
                                ("↑", Above)
                            } else if gap == file.diff.hunks.len() {
                                ("↓", Below)
                            } else {
                                ("↕", Both)
                            };
                            for (label, action, show) in [
                                (symbol, if small { All } else { expand }, remaining != Some(0)),
                                ("−", Collapse, context.before + context.after > 0),
                            ] {
                                if show {
                                    let id = WidgetId::new(("context", row.file, gap, action == Collapse));
                                    let mut button = controls.child(flex::item().fixed(sz::XXL, sz::XXL)).widget_id(id);
                                    let interaction = button.interact(id, Sense::CLICK);
                                    let lit = interaction.hovered || interaction.active;
                                    button.insert(Rectangle::new().background(if lit { theme::ACCENT } else { theme::SELECTED }));
                                    button.insert(
                                        widgets::text(
                                            label,
                                            theme::mono(sz::TEXT_BODY),
                                            if lit { theme::BACKGROUND } else { theme::ACCENT },
                                        )
                                        .options(TextOptions {
                                            horizontal_align: HorizontalAlign::Center,
                                            vertical_align: VerticalAlign::Center,
                                            ..TextOptions::default()
                                        }),
                                    );
                                    if interaction.clicked {
                                        let all = matches!(button.input(), Input::PointerUp { modifiers, .. } if modifiers.shift());
                                        context_action = Some((row.file, gap, if all && action != Collapse { All } else { action }));
                                        button.request_frame();
                                    }
                                }
                            }
                        }
                        drop(controls);
                        if let Some(Err(error)) = &file.source {
                            if context.is_some() {
                                bar.child(flex::item().grow()).insert(widgets::wrapped(error, theme::mono(sz::TEXT_SMALL), theme::MUTED));
                            }
                        } else if let Some(hunk) = heading {
                            bar.child(flex::item().grow()).insert(widgets::text(
                                &file.diff.hunks[hunk].header,
                                theme::mono(sz::CODE),
                                theme::MUTED,
                            ));
                        }
                    }
                    Kind::Header => {
                        if thread.is_some_and(|thread| thread.file == row.file && thread.button.is_none()) {
                            open_anchor = Some(WidgetId::new(("file comment", row.file)));
                        }
                        if let Some(action) = review::header(ui, row.file, file, &mut state.label) {
                            header_action = Some((row.file, action));
                        }
                    }
                    Kind::Code { left, right } => {
                        let mut pair = ui.layout(CodeLayout { split: *split });
                        pair.insert(Rectangle::new().background(theme::SURFACE));
                        for (source, side) in [(left, false), (right, true)].into_iter().take(if *split { 2 } else { 1 }) {
                            if side {
                                pair.child(CodeCell::Divider).insert(Rectangle::new().background(theme::BORDER));
                            }
                            let cell = pair.child(CodeCell::Line);
                            let Some(source) = source else { continue };
                            let side = (*split).then_some(side);
                            let (line, flat) = match source {
                                LineRef::Diff { hunk, line, flat } => (&file.diff.hunks[hunk].lines[line], Some(flat)),
                                LineRef::Source { gap, line } => {
                                    let Some(Ok(source)) = &file.source else { continue };
                                    (&source[gap][line], None)
                                }
                            };
                            let (mut highlighted, mut commented, mut show_plus) = (false, false, Some(false));
                            if let Some(flat) = flat {
                                let open = thread.is_some_and(|thread| thread.file == row.file && thread.button == Some(flat));
                                highlighted = selected.as_ref().is_some_and(|drag| {
                                    drag.file == row.file && (drag.start.min(drag.end)..=drag.start.max(drag.end)).contains(&flat)
                                }) || open;
                                commented = file.comments.iter().any(|comment| comment.at(Some(flat)));
                                show_plus = selected.map(|drag| drag.file == row.file && drag.end == flat).or_else(|| {
                                    (open
                                        || file
                                            .comments
                                            .iter()
                                            .any(|comment| matches!(comment.anchor, Anchor::Lines { end, .. } if end == flat)))
                                    .then_some(true)
                                });
                                if open && show_plus == Some(true) && (side != Some(false) || line.kind == Change::Del) {
                                    open_anchor = Some(WidgetId::new(("plus", row.file, flat)));
                                }
                            }
                            lines::line_row(cell, row.file, flat, line, highlighted, commented, show_plus, side, &mut started);
                        }
                    }
                    Kind::Gap | Kind::Empty => {}
                    Kind::Binary => {
                        let mut row = ui.layout(single::layout().padding(Sides::xy(sz::LG, sz::MD)));
                        row.child(single::item()).insert(widgets::text(
                            "Binary file, no diff.",
                            theme::interface(sz::TEXT_SMALL),
                            theme::MUTED,
                        ));
                    }
                }
            }),
    );
    if let (Some(selected), Some(Row { file, kind: Kind::Code { left, right } })) =
        (drag.as_mut(), response.pointer_row.map(|index| &state.rows[index]))
    {
        if *file == selected.file {
            if let Some(LineRef::Diff { flat, .. }) = if selected.side == Some(true) { right } else { left } {
                selected.end = *flat;
            }
        }
    }
    if let Some((index, gap, action)) = context_action {
        let file = &mut files[index];
        if file.source.is_none() {
            file.source = Some(
                file.diff
                    .source
                    .as_deref()
                    .ok_or_else(|| "Source unavailable".to_string())
                    .and_then(|source| crate::git::diff(&["show", source]))
                    .and_then(|source| file.diff.context(&source)),
            );
            if let Some(Ok(source)) = &file.source {
                for (context, lines) in file.context.iter_mut().zip(source) {
                    context.total = Some(lines.len());
                }
            }
        }
        if let Some(Ok(source)) = &file.source {
            let count = source[gap].len();
            let context = &mut file.context[gap];
            let first = context.before;
            use ContextAction::*;
            match action {
                Collapse => {
                    context.before = 0;
                    context.after = 0;
                }
                All => {
                    context.before = count;
                    context.after = 0;
                }
                _ => {
                    if matches!(action, Below | Both) {
                        context.before = (context.before + 20).min(count - context.after);
                    }
                    if matches!(action, Above | Both) {
                        context.after = (context.after + 20).min(count - context.before);
                    }
                }
            }
            let kind = if action == Collapse || count == 0 {
                Kind::Separator { heading: None, context: Some(gap) }
            } else {
                let line = Some(LineRef::Source {
                    gap,
                    line: match action {
                        Below | Both => first,
                        Above => count - context.after,
                        _ => 0,
                    },
                });
                Kind::Code { left: line, right: line }
            };
            state.list.scroll_to(Row { file: index, kind }.id(*split));
        }
        state.rows_dirty = true;
    }
    if let Some(file) = visible_file {
        *selected_file = file;
    }
    if let Some(started) = started {
        *drag = Some(started);
    }
    if released {
        if let Some(selected) = drag.take() {
            let end = selected.start.max(selected.end);
            let anchor = Anchor::Lines { start: selected.start.min(selected.end), end };
            let existing =
                selected.start == selected.end && files[selected.file].comments.iter().any(|comment| comment.at(Some(selected.end)));
            if !existing {
                files[selected.file].comments.push(Comment::new(anchor, String::new(), false));
            }
            *thread = Some(Thread { file: selected.file, line: Some(end), button: Some(selected.end) });
        }
    }
    if let Some((index, action)) = header_action {
        state.follow_scroll = false;
        *selected_file = index;
        let file = &mut files[index];
        let collapsed = file.collapsed;
        match action {
            HeaderAction::Collapse => file.collapsed = !file.collapsed,
            HeaderAction::Viewed => {
                file.viewed = !file.viewed;
                file.collapsed = file.viewed;
            }
            HeaderAction::Comment => {
                file.collapsed = false;
                let existing = !file.outdated.is_empty() || file.comments.iter().any(|comment| comment.anchor == Anchor::File);
                if !existing {
                    file.comments.push(Comment::new(Anchor::File, String::new(), false));
                }
                *thread = Some(Thread { file: index, line: None, button: None });
            }
        }
        if file.collapsed != collapsed {
            state.rows_dirty = true;
        }
    }
    open_anchor
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum LineRef {
    Diff { hunk: usize, line: usize, flat: usize },
    Source { gap: usize, line: usize },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ContextAction {
    Above,
    Below,
    Both,
    All,
    Collapse,
}

#[derive(Default)]
pub struct Context {
    pub total: Option<usize>,
    pub before: usize,
    pub after: usize,
}

impl Row {
    fn id(&self, split: bool) -> WidgetId {
        match self.kind {
            Kind::Separator { heading, context } => {
                WidgetId::new(("diff separator", split, self.file, context.or(heading), context.is_some()))
            }
            _ => WidgetId::new(("diff row", split, self.file, self.kind)),
        }
    }
}

struct CodeLayout {
    split: bool,
}

enum CodeCell {
    Line,
    Divider,
}

impl<R: Platform> Layout<R> for CodeLayout {
    type Item = CodeCell;

    fn layout(&self, ui: &mut LayoutCx<'_, R, CodeCell>, constraints: Constraints) -> Size {
        let gap = if self.split { ui.resolution().extent(Axis::Horizontal, sz::BORDER) } else { 0.0 };
        let width = (constraints.max.width - gap).max(0.0) / if self.split { 2.0 } else { 1.0 };
        let mut height = constraints.min.height;
        let mut x = 0.0;
        let mut divider = None;
        for child in ui.children() {
            if matches!(ui.item(child), CodeCell::Divider) {
                divider = Some(child);
                continue;
            }
            let size = ui.layout_child(child, Constraints { min: Size::new(width, 0.0), max: Size::new(width, constraints.max.height) });
            ui.set_child_position(child, Point::new(x, 0.0));
            height = height.max(size.height);
            x += width + gap;
        }
        let size = constraints.constrain(Size::new(constraints.max.width, height));
        if let Some(divider) = divider {
            ui.layout_child(divider, Constraints::tight(Size::new(gap, size.height)));
            ui.set_child_position(divider, Point::new(width, 0.0));
        }
        size
    }

    fn override_size(&self, _: &mut CodeCell, _: Option<f32>, _: Option<f32>) -> bool {
        false
    }
}
