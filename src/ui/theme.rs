//! warm graphite surfaces and restrained instrument colors

use blit_desktop::color::Color;
use blit_desktop::text::{FontId, TextStyle};

pub const TRANSITION: std::time::Duration = std::time::Duration::from_millis(140);

/// The interface face and the code face, as registered in `ui::fonts`.
pub const UI_FONT: FontId = FontId(0);
pub const MONO: FontId = FontId(1);

pub const BACKGROUND: Color = rgb(18, 19, 17);
pub const SURFACE: Color = rgb(25, 27, 24);
pub const RAISED: Color = rgb(38, 41, 35);
pub const BORDER: Color = rgb(65, 69, 59);
pub const TEXT: Color = rgb(227, 226, 211);
pub const MUTED: Color = rgb(151, 155, 137);
pub const ACCENT: Color = rgb(226, 174, 79);
pub const ACCENT_HOVER: Color = rgb(246, 201, 116);
pub const DANGER: Color = rgb(224, 127, 112);
pub const WARNING: Color = ACCENT;
pub const SUCCESS: Color = rgb(153, 188, 127);
pub const PURPLE: Color = rgb(180, 165, 190);

pub const ADD_LINE: Color = rgb(28, 37, 27);
pub const ADD_NUMBER: Color = rgb(37, 49, 32);
pub const DEL_LINE: Color = rgb(43, 29, 26);
pub const DEL_NUMBER: Color = rgb(57, 36, 30);
pub const HUNK: Color = rgb(34, 37, 31);
pub const SELECTED: Color = rgb(66, 54, 29);
pub const SHORTCUT_BACKGROUND: Color = rgba(0, 0, 0, 48);
pub const MARK_DANGER: Color = rgba(224, 127, 112, 85);
pub const MARK_WARNING: Color = rgba(226, 174, 79, 85);

pub mod sz {
    pub const BORDER: f32 = 1.0;
    pub const BORDER_STRONG: f32 = 2.0;
    pub const RADIUS: f32 = 0.0;

    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 16.0;
    pub const XXL: f32 = 24.0;
    pub const XXXL: f32 = 32.0;

    pub const TEXT_TINY: f32 = 10.0;
    pub const TEXT_LABEL: f32 = 11.0;
    pub const TEXT_SMALL: f32 = 12.0;
    pub const TEXT_BODY: f32 = 13.0;
    pub const TEXT_TITLE: f32 = 16.0;
    pub const CODE: f32 = 13.0;
    pub const LINE: f32 = 20.0;

    pub const CHECKBOX: f32 = 14.0;
    pub const SCROLLBAR: f32 = 10.0;
    pub const SIDEBAR: f32 = 280.0;
    pub const SIDEBAR_MIN: f32 = 80.0;
    pub const CONTEXT_WIDTH: f32 = 680.0;
    pub const HELP_WIDTH: f32 = 460.0;
    pub const POPOVER_WIDTH: f32 = 500.0;
    pub const POPOVER_HEIGHT: f32 = 260.0;
    pub const LINE_NUMBER: f32 = 48.0;
    pub const COMMENT_COLUMN: f32 = 22.0;
    pub const COMMENT_BAR: f32 = 3.0;
    pub const COMMENT_BUTTON: f32 = 18.0;
    pub const DIFF_MARKER: f32 = 14.0;
    pub const GUTTER: f32 = COMMENT_BAR + LINE_NUMBER * 2.0 + COMMENT_COLUMN;
}

const fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::from_rgba8(red, green, blue, 255)
}

const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Color {
    Color::from_rgba8(red, green, blue, alpha)
}

pub fn interface(size: f32) -> TextStyle {
    TextStyle { font: UI_FONT, size, ..TextStyle::default() }
}

pub fn bold(size: f32) -> TextStyle {
    TextStyle { font: UI_FONT, size, weight: 600, ..TextStyle::default() }
}

pub fn mono(size: f32) -> TextStyle {
    TextStyle { font: MONO, size, ..TextStyle::default() }
}
