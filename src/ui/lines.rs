//! diff rows and the floating comment thread

use blit::{Axis, Constraints, Layout, LayoutCx, Point, Sense, Sides, Size, Sizing, WidgetId};
use blit_gui::{
    Ui,
    atom::Rectangle,
    color::Color,
    layout::{flex, single, Align, Justify},
    style::BorderRadius,
    text::{HorizontalAlign, Span, TextOptions, TextWrap, VerticalAlign},
    widget::RichText,
};

use crate::diff::{Kind, Line};
use crate::text::{self, Anchor};

use super::review::{Comment, Drag, File, Outdated};
use super::syntax::Highlight;
use super::text_area::TextArea;
use super::theme::{self, sz};
use super::widgets::{self, panel, Button, Look, Tag};

/// What a click in the comments asks for, applied once the file is built.
enum Act {
    Delete(usize),
    Dismiss(usize),
    Reopen(usize),
    Close,
}

pub fn popover(ui: Ui<'_>, file: &mut File, line: Option<usize>, user: &str, consumed: &mut bool) -> bool {
    let mut act = None;
    {
        let mut area = ui.layout(flex::column().padding(Sides::all(sz::MD)).gap(sz::MD));
        area.insert(panel(theme::SURFACE));
        if line.is_none() {
            for (i, outdated) in file.outdated.iter().enumerate() {
                area.child().item(flex::item()).build(|ui: Ui<'_>| outdated_box(ui, &file.diff.path, i, outdated, user, &mut act));
            }
        }
        for (i, comment) in file.comments.iter_mut().enumerate().filter(|(_, comment)| comment.at(line)) {
            area.child().item(flex::item()).build(|ui: Ui<'_>| {
                let mut card = ui.layout(flex::column());
                card.insert(panel(theme::BACKGROUND));
                let id = comment.id;
                if comment.focus {
                    comment.focus = false;
                    card.focus(id);
                }
                let tags = [("Pending", theme::WARNING), ("Earlier round", theme::ACCENT_HOVER)];
                let place = text::location(&file.diff, comment.anchor);
                let place = place.strip_prefix(&file.diff.path).unwrap_or(&place).trim_start_matches(':');
                card.child().item(flex::item()).build(|ui: Ui<'_>| card_head(ui, user, &tags[..if comment.earlier { 2 } else { 1 }], place));
                let dismissed = card.child().item(flex::item()).build(|ui: Ui<'_>| {
                    let mut body = ui.layout(single::layout().padding(Sides::xy(sz::LG, sz::MD)));
                    body.child().item(single::item().width(Sizing::grow())).build(TextArea {
                        state: &mut comment.state,
                        id,
                        value: &mut comment.text,
                        placeholder: "Leave a comment",
                        rows: 3,
                    })
                });
                if dismissed {
                    *consumed = true;
                    act = Some(Act::Close);
                }
                match card.child().item(flex::item()).build(|ui: Ui<'_>| card_actions(ui, id, &[("Delete", Look::Plain), ("Done", Look::Plain)])) {
                    Some(0) => act = Some(Act::Delete(i)),
                    Some(_) => act = Some(Act::Close),
                    None => {}
                }
            });
        }
        if act.is_some() {
            area.request_frame();
        }
    }
    match act {
        Some(Act::Delete(i)) => {
            file.comments.remove(i);
            true
        }
        Some(Act::Dismiss(i)) => {
            file.outdated.remove(i);
            false
        }
        Some(Act::Reopen(i)) => {
            let outdated = file.outdated.remove(i);
            file.comments.push(Comment::new(Anchor::File, outdated.text, false));
            false
        }
        Some(Act::Close) => true,
        None => false,
    }
}

pub fn line_row(
    ui: Ui<'_>,
    index: usize,
    flat: Option<usize>,
    line: &Line,
    syntax: &[Highlight],
    highlighted: bool,
    commented: bool,
    show_plus: Option<bool>,
    side: Option<bool>,
    drag: &mut Option<Drag>,
) {
    let mut ui = ui;
    let row_id = match flat {
        Some(flat) => WidgetId::new(("line", index, flat, side)),
        None => WidgetId::new(("source line", index, line.old, side)),
    };
    let plus_id = WidgetId::new(("plus", index, flat.unwrap_or(0)));
    // rows only report the pointer over them
    let row = ui.interact(row_id, Sense::default());
    let interactive = flat.is_some() && (side != Some(false) || line.kind == Kind::Del);
    let plus = if interactive { ui.interact(plus_id, Sense::CLICK) } else { Default::default() };
    if plus.activated {
        let flat = flat.unwrap();
        *drag = Some(Drag { file: index, start: flat, end: flat, side });
        ui.request_frame();
    }
    let (line_color, number_color, marker) = match line.kind {
        Kind::Add => (theme::ADD_LINE, theme::ADD_NUMBER, "+"),
        Kind::Del => (theme::DEL_LINE, theme::DEL_NUMBER, "-"),
        Kind::Context => (theme::BACKGROUND, theme::SURFACE, " "),
    };
    let (line_color, number_color) = if highlighted { (theme::SELECTED, theme::SELECTED) } else { (line_color, number_color) };
    let mono = theme::mono(sz::CODE);
    let mut cells = ui.widget_id(row_id).layout(LineLayout { single: side.is_some() });
    cells.insert(Rectangle::new().background(line_color));
    cells.child().item(Cell::Numbers).insert(Rectangle::new().background(number_color));
    let bar = if commented { theme::WARNING } else { Color::TRANSPARENT };
    cells.child().item(Cell::Bar).insert(Rectangle::new().background(bar));
    let mut buffer = itoa::Buffer::new();
    for (cell, number) in [(Cell::Old, line.old), (Cell::New, line.new)] {
        if side.is_some_and(|right| right != matches!(cell, Cell::New)) {
            continue;
        }
        let shown = number.map_or("", |number| buffer.format(number));
        let options = TextOptions { horizontal_align: HorizontalAlign::Right, ..TextOptions::default() };
        cells.child().item(cell).insert(widgets::text(shown, mono, theme::MUTED).options(options));
    }
    if interactive && show_plus.unwrap_or(row.hovered || plus.hovered || plus.active) {
        let mut button = cells.child().item(Cell::Plus).widget_id(plus_id);
        button.insert(
            Rectangle::new()
                .background(if plus.hovered || plus.active { theme::ACCENT_HOVER } else { theme::ACCENT })
                .radius(BorderRadius::uniform(sz::RADIUS)),
        );
        let options =
            TextOptions { horizontal_align: HorizontalAlign::Center, vertical_align: VerticalAlign::Center, ..TextOptions::default() };
        button.insert(
            widgets::text(if commented && !highlighted { "•" } else { "+" }, theme::bold(sz::TEXT_BODY), theme::BACKGROUND)
                .options(options),
        );
    }
    cells.child().item(Cell::Marker).insert(widgets::text(marker, mono, theme::MUTED));
    let options = TextOptions { wrap: TextWrap::Character, ..TextOptions::default() };
    if syntax.is_empty() {
        cells.child().item(Cell::Code).insert(widgets::text(&line.text, mono, theme::TEXT).options(options));
    } else {
        let spans = syntax
            .iter()
            .filter_map(|highlight| {
                line.text
                    .get(highlight.range.clone())
                    .map(|text| Span::new(text).color(highlight.color))
            })
            .collect::<Vec<_>>();
        cells.child().item(Cell::Code).insert(RichText::new(&spans).style(mono).color(theme::TEXT).options(options));
    }
}

struct LineLayout {
    single: bool,
}

#[derive(Clone, Copy, Default)]
enum Cell {
    #[default]
    Code,
    Old,
    New,
    Marker,
    Plus,
    Bar,
    Numbers,
}

impl<P> Layout<P> for LineLayout {
    type Item = Cell;

    fn layout(&self, ui: &mut LayoutCx<'_, P, Cell>, constraints: Constraints) -> Size {
        let width = constraints.max.width;
        let res = ui.resolution();
        let bar = res.extent(Axis::Horizontal, sz::COMMENT_BAR);
        let number = res.extent(Axis::Horizontal, sz::LINE_NUMBER);
        let numbers_width = number * if self.single { 1.0 } else { 2.0 };
        let gutter = bar + numbers_width + res.extent(Axis::Horizontal, sz::COMMENT_COLUMN);
        let marker = res.extent(Axis::Horizontal, sz::DIFF_MARKER);
        let top = res.extent(Axis::Vertical, sz::XXS);
        let line_height = res.extent(Axis::Vertical, sz::LINE);
        let mut height = line_height;
        let mut numbers_height = top;
        let mut numbers = None;
        for child in ui.children() {
            let (x, y, width, bottom, fixed_height) = match ui.item(child) {
                Cell::Code => (gutter + marker, top, (width - gutter - marker - res.extent(Axis::Horizontal, sz::LG)).max(0.0), top, None),
                Cell::Old => (bar, top, (number - res.extent(Axis::Horizontal, sz::MD)).max(0.0), 0.0, None),
                Cell::New => {
                    (bar + if self.single { 0.0 } else { number }, top, (number - res.extent(Axis::Horizontal, sz::MD)).max(0.0), 0.0, None)
                }
                Cell::Marker => (gutter, top, marker, top, None),
                Cell::Plus => (
                    bar + numbers_width + res.extent(Axis::Horizontal, sz::XXS),
                    res.extent(Axis::Vertical, sz::BORDER),
                    res.extent(Axis::Horizontal, sz::COMMENT_BUTTON),
                    0.0,
                    Some(res.extent(Axis::Vertical, sz::COMMENT_BUTTON)),
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
            ui.layout_child(child, Constraints::tight(Size::new(numbers_width, numbers_height)));
            ui.set_child_position(child, Point::new(bar, 0.0));
        }
        constraints.constrain(Size::new(width, height))
    }

}

/// The head of a comment card: author, tags, and where it points.
fn card_head(ui: Ui<'_>, user: &str, tags: &[(&str, Color)], place: &str) {
    let mut head = ui.layout(flex::row().padding(Sides::xy(sz::LG, sz::SM)).gap(sz::MD).align(Align::Center));
    head.insert(Rectangle::new().background(theme::RAISED).radius(BorderRadius::new().top_left(sz::RADIUS).top_right(sz::RADIUS)));
    head.child().item(flex::item()).insert(widgets::text(user, theme::bold(sz::TEXT_SMALL), theme::TEXT));
    for &(label, color) in tags {
        head.child().item(flex::item()).build(Tag { label, color });
    }
    let options = TextOptions { horizontal_align: HorizontalAlign::Right, ..TextOptions::default() };
    head.child().item(flex::item().width(Sizing::grow())).insert(widgets::text(place, theme::mono(sz::CODE), theme::MUTED).options(options));
}

fn card_text(ui: Ui<'_>, value: &str) {
    let mut body = ui.layout(single::layout().padding(Sides::xy(sz::LG, sz::MD)));
    body.child().item(single::item().width(Sizing::grow())).insert(widgets::wrapped(value, theme::interface(sz::TEXT_BODY), theme::TEXT));
}

/// Buttons at the right of a card; returns the index of the one clicked.
fn card_actions(ui: Ui<'_>, id: WidgetId, labels: &[(&str, Look)]) -> Option<usize> {
    let mut row = ui.layout(flex::row().padding(Sides::new().left(sz::LG).right(sz::LG).bottom(sz::MD)).gap(sz::MD).justify(Justify::End));
    let mut clicked = None;
    for (i, &(label, look)) in labels.iter().enumerate() {
        let button = Button::new(id.child(i), label).look(look).style(theme::interface(sz::TEXT_SMALL));
        if row.child().item(flex::item()).build(button) {
            clicked = Some(i);
        }
    }
    clicked
}

fn outdated_box(ui: Ui<'_>, path: &str, i: usize, outdated: &Outdated, user: &str, act: &mut Option<Act>) {
    let mut card = ui.layout(flex::column());
    card.insert(panel(theme::BACKGROUND));
    card.child().item(flex::item()).build(|ui: Ui<'_>| card_head(ui, user, &[("Outdated", theme::MUTED)], path));
    if !outdated.quote.is_empty() {
        let quote = outdated.quote.join("\n");
        card.child().item(flex::item()).build(|ui: Ui<'_>| {
            let mut block = ui.layout(single::layout().padding(Sides::xy(sz::MD, sz::SM)));
            block.insert(panel(theme::SURFACE));
            block.child().item(single::item().width(Sizing::grow())).insert(widgets::text(&quote, theme::mono(sz::CODE), theme::MUTED));
        });
    }
    card.child().item(flex::item()).build(|ui: Ui<'_>| card_text(ui, &outdated.text));
    let id = WidgetId::new(("outdated", path, i));
    match card
        .child().item(flex::item())
        .build(|ui: Ui<'_>| card_actions(ui, id, &[("Dismiss", Look::Plain), ("Reopen", Look::Plain), ("Done", Look::Plain)]))
    {
        Some(0) => *act = Some(Act::Dismiss(i)),
        Some(1) => *act = Some(Act::Reopen(i)),
        Some(_) => *act = Some(Act::Close),
        None => {}
    }
}
