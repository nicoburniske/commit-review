//! Small widgets the views share: buttons, tags, a checkbox, scroll areas.

use blit::{Interaction, Sense, Sides, Widget, WidgetId};
use blit_desktop::atom::Rectangle;
use blit_desktop::color::Color;
use blit_desktop::layout::{flex, Align};
use blit_desktop::style::{Border, BorderRadius};
use blit_desktop::text::{HorizontalAlign, TextOptions, TextStyle, TextWrap, VerticalAlign};
use blit_desktop::widget::{scroll, Text};
use blit_desktop::{BoundsClip, DesktopPlatform, Ui};

use super::theme::{self, sz};

/// A line of text, clipped where it does not fit.
pub fn text(value: &str, style: TextStyle, color: Color) -> Text<'_> {
    Text::new(value).style(style).color(color)
}

/// Text that wraps at word boundaries to the width it is given.
pub fn wrapped(value: &str, style: TextStyle, color: Color) -> Text<'_> {
    text(value, style, color).options(TextOptions { wrap: TextWrap::Word, ..TextOptions::default() })
}

/// A bordered surface behind a group of content.
pub fn panel(background: Color) -> Rectangle {
    Rectangle::new().background(background).border(Border::solid(sz::BORDER, theme::BORDER)).radius(BorderRadius::uniform(sz::RADIUS))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Plain,
    Selected,
    Primary,
    Danger,
    Warning,
    /// No border until hovered: icons and links.
    Quiet,
}

/// A labeled button; its response is whether it was clicked.
pub struct Button<'a> {
    id: WidgetId,
    label: &'a str,
    look: Look,
    style: TextStyle,
    shortcut: Option<&'a str>,
}

impl<'a> Button<'a> {
    pub fn new(id: WidgetId, label: &'a str) -> Self {
        Self { id, label, look: Look::Plain, style: theme::bold(sz::TEXT_SMALL), shortcut: None }
    }

    pub fn look(mut self, look: Look) -> Self {
        self.look = look;
        self
    }

    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }
}

impl Widget<DesktopPlatform> for Button<'_> {
    type Response = bool;

    fn build(self, mut ui: Ui<'_>) -> bool {
        let interaction = ui.interact(self.id, Sense::CLICK);
        let (background, border, color) = colors(self.look, interaction);
        let padding = Sides::xy(sz::LG, sz::MD);
        let mut button = ui.widget_id(self.id).layout(flex::row().padding(padding).gap(sz::XL).align(Align::Center));
        button.insert(
            Rectangle::new()
                .background(background)
                .border(border.map_or(Border::None, |color| Border::solid(sz::BORDER, color)))
                .radius(BorderRadius::uniform(sz::RADIUS)),
        );
        button.child(flex::item()).insert(text(self.label, self.style, color));
        if let Some(shortcut) = self.shortcut {
            let color = if self.look == Look::Primary || interaction.active { color } else { theme::MUTED };
            let mut hint = button.child(flex::item()).layout(flex::row().padding(Sides::xy(sz::XS, sz::XXS)));
            hint.insert(
                Rectangle::new().background(theme::SHORTCUT_BACKGROUND).radius(BorderRadius::uniform(sz::XS)),
            );
            hint.child(flex::item()).insert(text(shortcut, theme::mono(sz::TEXT_TINY), color));
        }
        interaction.clicked
    }
}

fn colors(look: Look, interaction: Interaction) -> (Color, Option<Color>, Color) {
    if interaction.active {
        return if look == Look::Danger {
            (theme::DEL_NUMBER, Some(theme::DANGER), theme::DANGER)
        } else {
            (theme::SELECTED, Some(theme::ACCENT), theme::TEXT)
        };
    }
    let lit = interaction.hovered;
    match look {
        Look::Selected => (theme::SELECTED, Some(theme::ACCENT), theme::ACCENT),
        Look::Plain => {
            (if lit { theme::RAISED } else { theme::SURFACE }, Some(if lit { theme::ACCENT } else { theme::BORDER }), theme::TEXT)
        }
        Look::Primary => (if lit { theme::ACCENT_HOVER } else { theme::ACCENT }, Some(theme::ACCENT), theme::BACKGROUND),
        Look::Danger => (if lit { theme::DEL_NUMBER } else { theme::SURFACE }, Some(theme::DANGER), theme::DANGER),
        Look::Warning => (if lit { theme::SELECTED } else { theme::SURFACE }, Some(theme::WARNING), theme::WARNING),
        Look::Quiet => (if lit { theme::RAISED } else { Color::TRANSPARENT }, None, if lit { theme::ACCENT } else { theme::MUTED }),
    }
}

/// compact status label
pub struct Tag<'a> {
    pub label: &'a str,
    pub color: Color,
}

impl Widget<DesktopPlatform> for Tag<'_> {
    type Response = ();

    fn build(self, ui: Ui<'_>) {
        let mut tag = ui.layout(flex::row().padding(Sides::xy(sz::MD, sz::XXS)));
        tag.insert(Rectangle::new().background(theme::BACKGROUND).border(Border::solid(sz::BORDER, theme::BORDER)));
        tag.child(flex::item()).insert(text(self.label, theme::interface(sz::TEXT_LABEL), self.color));
    }
}

/// A checkbox with its label; its response is whether it was toggled.
pub struct Checkbox<'a> {
    pub id: WidgetId,
    pub label: &'a str,
    pub checked: bool,
}

impl Widget<DesktopPlatform> for Checkbox<'_> {
    type Response = bool;

    fn build(self, mut ui: Ui<'_>) -> bool {
        let interaction = ui.interact(self.id, Sense::CLICK);
        let mut row = ui.widget_id(self.id).layout(flex::row().padding(Sides::xy(sz::MD, sz::XS)).gap(sz::SM).align(Align::Center));
        row.insert(
            Rectangle::new()
                .background(if interaction.hovered { theme::RAISED } else { Color::TRANSPARENT })
                .radius(BorderRadius::uniform(sz::RADIUS)),
        );
        row.child(flex::item().fixed(sz::CHECKBOX, sz::CHECKBOX)).build(|ui: Ui<'_>| {
            let mut tick = ui;
            tick.insert(
                Rectangle::new()
                    .background(if self.checked { theme::ACCENT } else { Color::TRANSPARENT })
                    .border(Border::solid(sz::BORDER, if self.checked || interaction.hovered { theme::ACCENT } else { theme::MUTED })),
            );
            if self.checked {
                tick.insert(text("×", theme::bold(sz::TEXT_BODY), theme::BACKGROUND).options(TextOptions {
                    horizontal_align: HorizontalAlign::Center,
                    vertical_align: VerticalAlign::Center,
                    ..TextOptions::default()
                }));
            }
        });
        row.child(flex::item()).insert(text(self.label, theme::interface(sz::TEXT_SMALL), theme::TEXT));
        interaction.clicked
    }
}

/// a dedicated scrollbar rail with a square thumb
#[derive(Clone, Copy, Default)]
pub struct Thumb;

pub type ScrollArea<'a, C = ()> = scroll::Area<'a, DesktopPlatform, BoundsClip, Thumb, C>;
pub type VirtualList<'a, T, K = (), F = ()> = scroll::VirtualList<'a, DesktopPlatform, T, BoundsClip, Thumb, K, F>;

impl scroll::Scrollbar for Thumb {
    const HAS_TRACK: bool = true;
    const HAS_THUMB: bool = true;

    type Track = Rectangle;
    type Thumb = Rectangle;

    fn config(&self) -> scroll::Config {
        scroll::Config::new().scroll_speed(3.0).scrollbar_thickness(sz::SCROLLBAR).minimum_thumb_extent(sz::XXXL)
    }

    fn into_content(self, active: bool) -> (Self::Track, Self::Thumb) {
        let color = if active { theme::ACCENT } else { theme::MUTED };
        (
            Rectangle::new().background(theme::BACKGROUND).border(Border::solid(sz::BORDER, theme::BORDER)),
            Rectangle::new().background(color).border(Border::solid(sz::BORDER_STRONG, theme::BACKGROUND)),
        )
    }
}
