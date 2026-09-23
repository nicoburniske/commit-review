//! Text with highlighted spans: the findings of the commit message, and the
//! selection of a text area.

use std::ops::Range;

use blit::{state, Atom, Constraints, Content, LogicalRect, Size};
use blit_gui::{
    GuiContext, Ui,
    color::Color,
    display_list::Rectangle as Fill,
    text::{TextLayoutRequest, TextOptions, TextRequest, TextRunId, TextStyle, TextWrap},
};

use crate::message::Kind;

use super::theme;

/// Text whose marks, byte ranges of `text`, get a tinted background.
pub struct Marked<'a> {
    pub text: &'a str,
    pub marks: &'a [(Range<usize>, Kind)],
    pub style: TextStyle,
    pub wrap: TextWrap,
}

impl Content<GuiContext> for Marked<'_> {
    type Response = ();

    fn append(self, mut ui: Ui<'_, state::Node>) {
        let run = ui.context().text_run(self.text, self.style);
        let marks = self
            .marks
            .iter()
            .map(|(range, kind)| {
                let color = if *kind == Kind::NonAscii { theme::MARK_DANGER } else { theme::MARK_WARNING };
                (boundaries(self.text, range.clone()), color)
            })
            .collect();
        ui.insert(MarkedAtom { run, wrap: self.wrap, marks });
    }
}

struct MarkedAtom {
    run: TextRunId,
    wrap: TextWrap,
    /// Character boundaries of each mark, and its tint.
    marks: Vec<(Vec<usize>, Color)>,
}

impl Atom<GuiContext> for MarkedAtom {
    fn measure(&self, platform: &mut GuiContext, constraints: Constraints) -> Size {
        let wraps = self.wrap != TextWrap::None && constraints.max.width.is_finite();
        constraints.constrain(platform.measure_text(&TextLayoutRequest {
            text: self.run,
            wrap: self.wrap,
            max_width: wraps.then_some(constraints.max.width),
            max_lines: None,
        }))
    }

    fn paint(&self, platform: &mut GuiContext, area: LogicalRect) {
        let request = TextRequest {
            text: self.run,
            area,
            offset_x: 0.0,
            color: theme::TEXT,
            options: TextOptions { wrap: self.wrap, ..TextOptions::default() },
        };
        for (offsets, color) in &self.marks {
            for rect in span_rects(platform, &request, offsets) {
                platform.paint_rectangle(Fill::new(rect).background(*color));
            }
        }
        platform.paint_text(request);
    }

    fn paint_bounds(&self, area: LogicalRect) -> LogicalRect {
        area
    }
}

/// Byte offsets of every character boundary in `range`, both ends included.
pub fn boundaries(text: &str, range: Range<usize>) -> Vec<usize> {
    let Some(slice) = text.get(range.clone()) else {
        return Vec::new();
    };
    slice
        .char_indices()
        .map(|(offset, _)| range.start + offset)
        .chain(std::iter::once(range.end))
        .collect()
}

/// One rectangle per laid-out line the offsets cross, from caret to caret.
pub fn span_rects(platform: &mut GuiContext, request: &TextRequest, offsets: &[usize]) -> Vec<LogicalRect> {
    let mut rects: Vec<LogicalRect> = Vec::new();
    for &offset in offsets {
        let caret = platform.text_cursor_rect(request, offset);
        match rects.last_mut() {
            Some(rect) if rect.y == caret.y => {
                let right = (rect.x + rect.width).max(caret.x);
                rect.x = rect.x.min(caret.x);
                rect.width = right - rect.x;
            }
            _ => rects.push(LogicalRect::new(caret.x, caret.y, 0.0, caret.height)),
        }
    }
    rects.retain(|rect| rect.width > 0.0);
    rects
}
