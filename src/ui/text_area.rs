//! A multi-line text field for the notes and the comments. blit's TextInput
//! holds one line; this one wraps, takes Enter as a newline, moves between
//! lines with the arrows and reaches the clipboard through the system tools.

use std::io::Write as _;
use std::process::{Command, Stdio};

use blit::{Atom, Constraints, Input, Key, LogicalRect, Point, PointerButton, Sense, Size, Widget, WidgetId};
use blit_gui::{
    GuiContext, Ui,
    display_list::Rectangle as Fill,
    text::{TextLayoutRequest, TextOptions, TextRequest, TextRunId, TextWrap},
    widget::text_input::State,
};

use super::marked::{boundaries, span_rects};
use super::theme::{self, sz};

pub struct TextArea<'a> {
    pub state: &'a mut State,
    pub id: WidgetId,
    pub value: &'a mut String,
    pub placeholder: &'a str,
    /// Lines of height kept while the text is shorter.
    pub rows: u16,
}

impl Widget<GuiContext> for TextArea<'_> {
    type Response = bool;

    fn build(self, mut ui: Ui<'_>) -> bool {
        let Self { state, id, value, placeholder, rows } = self;
        let interaction = ui.interact(id, Sense::FOCUS);
        let focused = ui.is_focused(id);
        let input = *ui.input();
        let style = theme::interface(sz::TEXT_BODY);
        let options = TextOptions { wrap: TextWrap::Character, ..TextOptions::default() };
        let mut close = false;
        let mut changed = false;
        let mut vertical = None;
        if focused {
            match input {
                Input::Key(key) if key.pressed => {
                    let command = key.modifiers.control() || key.modifiers.super_key();
                    match key.key {
                        Key::Escape => close = true,
                        Key::Enter if command => close = true,
                        Key::Enter => changed = insert(state, value, "\n"),
                        Key::ArrowUp => vertical = Some((false, key.modifiers.shift())),
                        Key::ArrowDown => vertical = Some((true, key.modifiers.shift())),
                        Key::Character('v' | 'V') if command => {
                            if let Some(pasted) = paste() {
                                changed = insert(state, value, &pasted.replace("\r\n", "\n"));
                            }
                        }
                        Key::Character('c' | 'C' | 'x' | 'X') if command => {
                            let (start, end) = selection(state, value);
                            if start != end {
                                copy(&value[start..end]);
                                if matches!(key.key, Key::Character('x' | 'X')) {
                                    changed = insert(state, value, "");
                                }
                            }
                        }
                        _ => changed = state.update(value, &input).changed,
                    }
                }
                _ => changed = state.update(value, &input).changed,
            }
        }
        let text = ui.context().text_run(value, style);
        if let Some(area) = ui.geometry(id) {
            let request = TextRequest { text, area, offset_x: 0.0, color: theme::TEXT, options };
            if let Some((down, extend)) = vertical {
                let caret = ui.context().text_cursor_rect(&request, state.cursor);
                let y = if down { caret.y + caret.height * 1.5 } else { caret.y - caret.height * 0.5 };
                let offset = ui.context().text_offset_at_position(&request, Point::new(caret.x, y));
                state.move_to(value, offset, extend);
            }
            let pointer = match input {
                Input::PointerDown { position, button: PointerButton::Primary, modifiers } if interaction.active => {
                    Some((position, modifiers.shift()))
                }
                Input::PointerMove { position, .. } if interaction.active => Some((position, true)),
                _ => None,
            };
            if let (true, Some((position, extend))) = (focused, pointer) {
                let offset = ui.context().text_offset_at_position(&request, position);
                state.move_to(value, offset, extend);
            }
        } else {
            ui.request_frame();
        }
        if changed {
            ui.request_frame();
        }
        let (start, end) = selection(state, value);
        let empty = value.is_empty();
        let display = if empty { ui.context().text_run(placeholder, style) } else { text };
        ui.widget_id(id).insert(Field {
            text,
            display,
            placeholder: empty,
            cursor: state.cursor,
            selection: if start == end { Vec::new() } else { boundaries(value, start..end) },
            focused,
            options,
            min_height: style.size * 1.2 * f32::from(rows),
        });
        close
    }
}

fn selection(state: &State, value: &str) -> (usize, usize) {
    let len = value.len();
    (state.cursor.min(state.anchor).min(len), state.cursor.max(state.anchor).min(len))
}

/// Replaces the selection with `text` and puts the cursor after it.
fn insert(state: &mut State, value: &mut String, text: &str) -> bool {
    let (start, end) = selection(state, value);
    value.replace_range(start..end, text);
    state.cursor = start + text.len();
    state.anchor = state.cursor;
    start != end || !text.is_empty()
}

/// The clipboard as text, or nothing when the tool is missing.
fn paste() -> Option<String> {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") { ("pbpaste", &[]) } else { ("wl-paste", &["--no-newline"]) };
    let out = Command::new(program).args(args).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn copy(text: &str) {
    let program = if cfg!(target_os = "macos") { "pbcopy" } else { "wl-copy" };
    let Ok(mut child) = Command::new(program).stdin(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
}

struct Field {
    text: TextRunId,
    display: TextRunId,
    placeholder: bool,
    cursor: usize,
    /// Character boundaries of the selection, empty when there is none.
    selection: Vec<usize>,
    focused: bool,
    options: TextOptions,
    min_height: f32,
}

impl Atom<GuiContext> for Field {
    fn measure(&self, platform: &mut GuiContext, constraints: Constraints) -> Size {
        let size = platform.measure_text(&TextLayoutRequest {
            text: self.display,
            wrap: self.options.wrap,
            max_width: constraints.max.width.is_finite().then_some(constraints.max.width),
            max_lines: None,
        });
        constraints.constrain(Size::new(size.width, size.height.max(self.min_height)))
    }

    fn paint(&self, platform: &mut GuiContext, area: LogicalRect) {
        let request = TextRequest { text: self.text, area, offset_x: 0.0, color: theme::TEXT, options: self.options };
        for rect in span_rects(platform, &request, &self.selection) {
            platform.paint_rectangle(Fill::new(rect).background(theme::SELECTED));
        }
        if self.focused {
            let caret = platform.text_cursor_rect(&request, self.cursor);
            let caret = LogicalRect::new(caret.x, caret.y, caret.width.max(sz::BORDER), caret.height);
            platform.paint_rectangle(Fill::new(caret).background(theme::TEXT));
        }
        let color = if self.placeholder { theme::MUTED } else { theme::TEXT };
        platform.paint_text(TextRequest { text: self.display, color, ..request });
    }

    fn paint_bounds(&self, area: LogicalRect) -> LogicalRect {
        area
    }
}
