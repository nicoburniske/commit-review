//! Below a file's header: its comments and its diff. Each line has a "+" to
//! comment on it, dragged to select a range; comments and the comment form
//! sit under the last line they quote.

use blit::{state, Axis, Constraints, Layout, LayoutCx, Platform, Point, Sense, Sides, Size, Sizing, WidgetId};
use blit_desktop::atom::Rectangle;
use blit_desktop::color::Color;
use blit_desktop::layout::{flex, single, Align, Justify};
use blit_desktop::style::BorderRadius;
use blit_desktop::text::{HorizontalAlign, TextOptions, TextWrap, VerticalAlign};
use blit_desktop::Ui;

use crate::diff::{Kind, Line};
use crate::text::{self, Anchor};

use super::review::{Comment, Drag, File, Form, Outdated};
use super::text_area::TextArea;
use super::theme;
use super::widgets::{self, panel, Button, Look, Tag};

/// Width of a line number column.
const NUMBER: f32 = 48.0;
/// Width of the column the "+" appears in.
const PLUS: f32 = 22.0;
/// Width of the bar marking commented lines.
const BAR: f32 = 3.0;

/// What a click in the comments asks for, applied once the file is built.
enum Act {
    Edit(usize),
    Delete(usize),
    Dismiss(usize),
    Reopen(usize),
    Submit,
    Cancel,
}

pub(super) fn body(
    ui: Ui<'_>,
    index: usize,
    file: &mut File,
    drag: &mut Option<Drag>,
    form: &mut Option<Form>,
    user: &str,
    consumed: &mut bool,
) {
    let mut acts = Vec::new();
    {
        let file: &File = file;
        let mut column = ui.layout(flex::column());
        if file.comments.iter().any(|comment| sits(comment.anchor, None))
            || !file.outdated.is_empty()
            || form.as_ref().is_some_and(|open| open.file == index && sits(open.anchor, None))
        {
            column.child(flex::item()).build(|ui: Ui<'_>| {
                let mut area = thread_area(ui);
                for (i, outdated) in file.outdated.iter().enumerate() {
                    area.child(flex::item()).build(|ui: Ui<'_>| outdated_box(ui, &file.diff.path, i, outdated, user, &mut acts));
                }
                thread(&mut area, index, file, None, form, user, consumed, &mut acts);
            });
        }
        if file.diff.binary {
            let note = widgets::text("Binary file, no diff.", theme::sans(theme::SMALL), theme::MUTED);
            column.child(flex::item()).build(|ui: Ui<'_>| {
                let mut row = ui.layout(single::layout().padding(Sides::xy(12.0, 10.0)));
                row.child(single::item()).insert(note);
            });
        }
        let selected = drag.filter(|drag| drag.file == index).map(|drag| (drag.start.min(drag.end), drag.start.max(drag.end)));
        let mut flat = 0;
        for hunk in &file.diff.hunks {
            column.child(flex::item()).build(|ui: Ui<'_>| {
                let padding = Sides::new().left(BAR + NUMBER * 2.0 + PLUS).top(2.0).bottom(2.0);
                let mut row = ui.layout(flex::row().padding(padding));
                row.insert(Rectangle::new().background(theme::HUNK));
                row.child(flex::item().width(Sizing::grow())).insert(widgets::text(&hunk.header, theme::mono(theme::CODE), theme::MUTED));
            });
            for line in &hunk.lines {
                let highlighted = selected.is_some_and(|(start, end)| (start..=end).contains(&flat));
                let commented = file
                    .comments
                    .iter()
                    .any(|comment| matches!(comment.anchor, Anchor::Lines { start, end } if (start..=end).contains(&flat)));
                column.child(flex::item()).build(|ui: Ui<'_>| line_row(ui, index, flat, line, highlighted, commented, drag));
                let has_thread = file.comments.iter().any(|comment| sits(comment.anchor, Some(flat)))
                    || form.as_ref().is_some_and(|open| open.file == index && sits(open.anchor, Some(flat)));
                if has_thread {
                    column.child(flex::item()).build(|ui: Ui<'_>| {
                        let mut area = thread_area(ui);
                        thread(&mut area, index, file, Some(flat), form, user, consumed, &mut acts);
                    });
                }
                flat += 1;
            }
        }
    }
    apply(acts, index, file, form);
}

/// Whether a comment anchored here belongs under `line`, or on the file
/// when there is no line.
fn sits(anchor: Anchor, line: Option<usize>) -> bool {
    match (anchor, line) {
        (Anchor::File, None) => true,
        (Anchor::Lines { end, .. }, Some(line)) => end == line,
        _ => false,
    }
}

fn apply(acts: Vec<Act>, index: usize, file: &mut File, form: &mut Option<Form>) {
    for act in acts {
        match act {
            Act::Edit(i) => {
                let comment = &file.comments[i];
                *form = Some(Form::new(index, comment.anchor, comment.text.clone(), Some(i)));
            }
            Act::Delete(i) => {
                file.comments.remove(i);
                if form.as_ref().is_some_and(|open| open.file == index && open.editing.is_some()) {
                    *form = None;
                }
            }
            Act::Dismiss(i) => {
                file.outdated.remove(i);
            }
            Act::Reopen(i) => {
                let outdated = file.outdated.remove(i);
                file.comments.push(Comment { anchor: Anchor::File, text: outdated.text, earlier: false });
            }
            Act::Submit => {
                let Some(open) = form.take() else { continue };
                let text = open.text.trim().to_string();
                match (text.is_empty(), open.editing) {
                    (true, _) => *form = Some(open),
                    (false, Some(i)) => {
                        if let Some(comment) = file.comments.get_mut(i) {
                            comment.text = text;
                        }
                    }
                    (false, None) => file.comments.push(Comment { anchor: open.anchor, text, earlier: false }),
                }
            }
            Act::Cancel => *form = None,
        }
    }
}

fn line_row(ui: Ui<'_>, index: usize, flat: usize, line: &Line, highlighted: bool, commented: bool, drag: &mut Option<Drag>) {
    let mut ui = ui;
    let row_id = WidgetId::new(("line", index, flat));
    let plus_id = WidgetId::new(("plus", index, flat));
    // Rows sense nothing: they only report the pointer over them.
    let row = ui.interact(row_id, Sense::default());
    let plus = ui.interact(plus_id, Sense::CLICK);
    if plus.activated {
        *drag = Some(Drag { file: index, start: flat, end: flat });
    }
    if let Some(drag) = drag.as_mut().filter(|drag| drag.file == index) {
        if row.hovered || plus.hovered {
            drag.end = flat;
        }
    }
    let (line_color, number_color, marker) = match line.kind {
        Kind::Add => (theme::ADD_LINE, theme::ADD_NUMBER, "+"),
        Kind::Del => (theme::DEL_LINE, theme::DEL_NUMBER, "-"),
        Kind::Context => (Color::TRANSPARENT, Color::TRANSPARENT, " "),
    };
    let (line_color, number_color) = if highlighted { (theme::SELECTED, theme::SELECTED) } else { (line_color, number_color) };
    let mono = theme::mono(theme::CODE);
    let mut cells = ui.widget_id(row_id).layout(LineLayout);
    cells.insert(Rectangle::new().background(line_color));
    cells.child(Cell::Numbers).insert(Rectangle::new().background(number_color));
    let bar = if commented { theme::WARNING } else { Color::TRANSPARENT };
    cells.child(Cell::Bar).insert(Rectangle::new().background(bar));
    let mut buffer = itoa::Buffer::new();
    for (cell, number) in [(Cell::Old, line.old), (Cell::New, line.new)] {
        let shown = number.map_or("", |number| buffer.format(number));
        let options = TextOptions { horizontal_align: HorizontalAlign::Right, ..TextOptions::default() };
        cells.child(cell).insert(widgets::text(shown, mono, theme::MUTED).options(options));
    }
    if row.hovered || plus.hovered || plus.active {
        let mut button = cells.child(Cell::Plus).widget_id(plus_id);
        button.insert(Rectangle::new().background(theme::ACCENT).radius(BorderRadius::uniform(4.0)));
        let options =
            TextOptions { horizontal_align: HorizontalAlign::Center, vertical_align: VerticalAlign::Center, ..TextOptions::default() };
        button.insert(widgets::text("+", theme::bold(14.0), theme::WHITE).options(options));
    }
    cells.child(Cell::Marker).insert(widgets::text(marker, mono, theme::MUTED));
    let options = TextOptions { wrap: TextWrap::Character, ..TextOptions::default() };
    cells.child(Cell::Code).insert(widgets::text(&line.text, mono, theme::TEXT).options(options));
}

struct LineLayout;

#[derive(Clone, Copy)]
enum Cell {
    Code,
    Old,
    New,
    Marker,
    Plus,
    Bar,
    Numbers,
}

impl<P: Platform> Layout<P> for LineLayout {
    type Item = Cell;

    fn layout(&self, ui: &mut LayoutCx<'_, P, Cell>, constraints: Constraints) -> Size {
        let width = constraints.max.width;
        let res = ui.resolution();
        let bar = res.extent(Axis::Horizontal, BAR);
        let number = res.extent(Axis::Horizontal, NUMBER);
        let gutter = bar + number * 2.0 + res.extent(Axis::Horizontal, PLUS);
        let marker = res.extent(Axis::Horizontal, 14.0);
        let top = res.extent(Axis::Vertical, 2.0);
        let line_height = res.extent(Axis::Vertical, theme::LINE);
        let mut height = line_height;
        let mut numbers_height = top;
        let mut numbers = None;
        for child in ui.children() {
            let (x, y, width, bottom, fixed_height) = match ui.item(child) {
                Cell::Code => (gutter + marker, top, (width - gutter - marker - res.extent(Axis::Horizontal, 12.0)).max(0.0), top, None),
                Cell::Old => (bar, top, (number - res.extent(Axis::Horizontal, 8.0)).max(0.0), 0.0, None),
                Cell::New => (bar + number, top, (number - res.extent(Axis::Horizontal, 8.0)).max(0.0), 0.0, None),
                Cell::Marker => (gutter, top, marker, top, None),
                Cell::Plus => (
                    bar + number * 2.0 + res.extent(Axis::Horizontal, 2.0),
                    res.extent(Axis::Vertical, 1.0),
                    res.extent(Axis::Horizontal, 18.0),
                    0.0,
                    Some(res.extent(Axis::Vertical, 18.0)),
                ),
                Cell::Bar => (0.0, 0.0, bar, 0.0, Some(line_height)),
                Cell::Numbers => {
                    numbers = Some(child);
                    continue;
                }
            };
            let bounds = Constraints {
                min: Size::new(width, fixed_height.unwrap_or(0.0)),
                max: Size::new(width, fixed_height.unwrap_or((constraints.max.height - y - bottom).max(0.0))),
            };
            let size = ui.layout_child(child, bounds);
            ui.set_child_position(child, Point::new(x, y));
            height = height.max(y + size.height + bottom);
            if matches!(ui.item(child), Cell::Old | Cell::New) {
                numbers_height = numbers_height.max(y + size.height);
            }
        }
        if let Some(child) = numbers {
            ui.layout_child(child, Constraints::tight(Size::new(number * 2.0, numbers_height)));
            ui.set_child_position(child, Point::new(bar, 0.0));
        }
        constraints.constrain(Size::new(width, height))
    }

    fn override_size(&self, _: &mut Cell, _: Option<f32>, _: Option<f32>) -> bool {
        false
    }
}

fn thread_area(ui: Ui<'_>) -> Ui<'_, state::Open<flex::Layout>> {
    let padding = Sides::new().top(8.0).right(12.0).bottom(8.0).left(BAR + NUMBER * 2.0 + PLUS);
    let mut area = ui.layout(flex::column().padding(padding).gap(8.0));
    area.insert(Rectangle::new().background(theme::SURFACE));
    area
}

#[allow(clippy::too_many_arguments)]
fn thread(
    area: &mut Ui<'_, state::Open<flex::Layout>>,
    index: usize,
    file: &File,
    line: Option<usize>,
    form: &mut Option<Form>,
    user: &str,
    consumed: &mut bool,
    acts: &mut Vec<Act>,
) {
    let editing = form.as_ref().filter(|open| open.file == index).and_then(|open| open.editing);
    for (i, comment) in file.comments.iter().enumerate() {
        if sits(comment.anchor, line) && editing != Some(i) {
            area.child(flex::item()).build(|ui: Ui<'_>| comment_box(ui, file, i, comment, user, acts));
        }
    }
    if let Some(open) = form.as_mut().filter(|open| open.file == index && sits(open.anchor, line)) {
        area.child(flex::item()).build(|ui: Ui<'_>| form_box(ui, file, open, consumed, acts));
    }
}

/// The head of a comment card: author, tags, and where it points.
fn card_head(ui: Ui<'_>, user: &str, tags: &[(&str, Color)], place: &str) {
    let mut head = ui.layout(flex::row().padding(Sides::xy(12.0, 6.0)).gap(8.0).align(Align::Center));
    head.insert(Rectangle::new().background(theme::RAISED).radius(BorderRadius::new().top_left(theme::RADIUS).top_right(theme::RADIUS)));
    head.child(flex::item()).insert(widgets::text(user, theme::bold(theme::SMALL), theme::TEXT));
    for &(label, color) in tags {
        head.child(flex::item()).build(Tag { label, color });
    }
    let options = TextOptions { horizontal_align: HorizontalAlign::Right, ..TextOptions::default() };
    head.child(flex::item().width(Sizing::grow())).insert(widgets::text(place, theme::mono(theme::CODE), theme::MUTED).options(options));
}

fn card_text(ui: Ui<'_>, value: &str) {
    let mut body = ui.layout(single::layout().padding(Sides::xy(12.0, 10.0)));
    body.child(single::item().width(Sizing::grow())).insert(widgets::wrapped(value, theme::sans(13.0), theme::TEXT));
}

/// Buttons at the right of a card; returns the index of the one clicked.
fn card_actions(ui: Ui<'_>, id: WidgetId, labels: &[(&str, Look)]) -> Option<usize> {
    let mut row = ui.layout(flex::row().padding(Sides::new().left(12.0).right(12.0).bottom(8.0)).gap(8.0).justify(Justify::End));
    let mut clicked = None;
    for (i, &(label, look)) in labels.iter().enumerate() {
        let button = Button::new(id.child(i), label).look(look).style(theme::sans(theme::SMALL));
        if row.child(flex::item()).build(button) {
            clicked = Some(i);
        }
    }
    clicked
}

fn comment_box(ui: Ui<'_>, file: &File, i: usize, comment: &Comment, user: &str, acts: &mut Vec<Act>) {
    let mut card = ui.layout(flex::column());
    card.insert(panel(theme::BACKGROUND));
    let mut tags = vec![("Pending", theme::WARNING)];
    if comment.earlier {
        tags.push(("Earlier round", theme::ACCENT_HOVER));
    }
    let place = text::location(&file.diff, comment.anchor);
    card.child(flex::item()).build(|ui: Ui<'_>| card_head(ui, user, &tags, &place));
    card.child(flex::item()).build(|ui: Ui<'_>| card_text(ui, &comment.text));
    let id = WidgetId::new(("comment", &file.diff.path, i));
    match card.child(flex::item()).build(|ui: Ui<'_>| card_actions(ui, id, &[("Edit", Look::Plain), ("Delete", Look::Plain)])) {
        Some(0) => acts.push(Act::Edit(i)),
        Some(_) => acts.push(Act::Delete(i)),
        None => {}
    }
}

fn outdated_box(ui: Ui<'_>, path: &str, i: usize, outdated: &Outdated, user: &str, acts: &mut Vec<Act>) {
    let mut card = ui.layout(flex::column());
    card.insert(panel(theme::BACKGROUND));
    card.child(flex::item()).build(|ui: Ui<'_>| card_head(ui, user, &[("Outdated", theme::MUTED)], path));
    if !outdated.quote.is_empty() {
        let quote = outdated.quote.join("\n");
        card.child(flex::item()).build(|ui: Ui<'_>| {
            let mut block = ui.layout(single::layout().padding(Sides::xy(10.0, 6.0)));
            block.insert(panel(theme::SURFACE));
            block.child(single::item().width(Sizing::grow())).insert(widgets::text(&quote, theme::mono(theme::CODE), theme::MUTED));
        });
    }
    card.child(flex::item()).build(|ui: Ui<'_>| card_text(ui, &outdated.text));
    let id = WidgetId::new(("outdated", path, i));
    match card.child(flex::item()).build(|ui: Ui<'_>| card_actions(ui, id, &[("Dismiss", Look::Plain), ("Reopen", Look::Plain)])) {
        Some(0) => acts.push(Act::Dismiss(i)),
        Some(_) => acts.push(Act::Reopen(i)),
        None => {}
    }
}

fn form_box(ui: Ui<'_>, file: &File, form: &mut Form, consumed: &mut bool, acts: &mut Vec<Act>) {
    let id = WidgetId::new("comment form");
    let mut card = ui.layout(flex::column().padding(Sides::new().top(8.0)).gap(8.0));
    card.insert(panel(theme::BACKGROUND));
    if form.focus {
        form.focus = false;
        card.focus(id);
    }
    let place = text::location(&file.diff, form.anchor);
    card.child(flex::item()).build(|ui: Ui<'_>| {
        let mut row = ui.layout(single::layout().padding(Sides::x(12.0)));
        row.child(single::item()).insert(widgets::text(&place, theme::mono(theme::CODE), theme::MUTED));
    });
    let response = card.child(flex::item()).build(|ui: Ui<'_>| {
        let mut outer = ui.layout(single::layout().padding(Sides::x(12.0)));
        outer.child(single::item().width(Sizing::grow())).build(|ui: Ui<'_>| {
            let mut field = ui.layout(single::layout().padding(Sides::xy(10.0, 6.0)));
            field.insert(panel(theme::SURFACE));
            field.child(single::item().width(Sizing::grow())).build(TextArea {
                state: &mut form.state,
                id,
                value: &mut form.text,
                placeholder: "Leave a comment",
                rows: 3,
            })
        })
    });
    let submit = if form.editing.is_some() { "Update comment" } else { "Add review comment" };
    let clicked = card
        .child(flex::item())
        .build(|ui: Ui<'_>| card_actions(ui, WidgetId::new("comment form actions"), &[("Cancel", Look::Plain), (submit, Look::Primary)]));
    if response.escaped || clicked == Some(0) {
        *consumed |= response.escaped;
        acts.push(Act::Cancel);
    } else if response.submitted || clicked == Some(1) {
        *consumed |= response.submitted;
        acts.push(Act::Submit);
    }
}
