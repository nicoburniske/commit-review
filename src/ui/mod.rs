//! diff first review with commit context, feedback and a final decision

mod diff_view;
mod lines;
mod marked;
mod review;
mod summary;
mod syntax;
mod text_area;
mod theme;
mod widgets;

use std::cell::RefCell;

use blit::{Absolute, Anchor, Input, Key, Point, PointerButton, Sense, Sides, Sizing, WidgetId};
use blit_desktop::{gpu, Application, Config, EventLoopProxy, Root};
use blit_gui::{
    FontFamily, GuiContext, TextConfig, TextLayoutEngine, Ui,
    atom::Rectangle,
    layout::{flex, Align, Justify},
    widget::{performance, popover, scroll_area, text_input, Performance},
};
use blit_text::{FontFaceId, FontStyle, SystemFontRequest};

use crate::{Context, Output};
use text_area::TextArea;
use theme::sz;
use widgets::{panel as panel_surface, text, Button, Look};

thread_local! {
    /// What `run` hands over to the app, which blit builds without arguments.
    static LAUNCH: RefCell<Option<(Option<String>, Output)>> = const { RefCell::new(None) };
}

/// Candidate families, the first one installed wins: interface, then code.
const MONO: &[&str] = &[
    "Berkeley Mono",
    "Berkeley Mono Variable",
    "IBM Plex Mono",
    "JetBrains Mono",
    "Fira Code",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
    "Liberation Mono",
    "SF Mono",
    "Menlo",
    "Monaco",
];

const ACCEPT_SHORTCUT: &str = if cfg!(target_os = "macos") { "⌘ ↵" } else { "Ctrl ↵" };

/// Opens the window. Every decision leaves the process from inside it, so
/// this returns only when the window is closed without one.
pub fn run(command: Option<String>, output: Output) -> Result<(), String> {
    let mut engine = blit_text_cosmic::Backend::new();
    let fonts = fonts(&mut engine)?;
    LAUNCH.with(|launch| *launch.borrow_mut() = Some((command, output)));
    blit_desktop::run::<App>(Config {
        title: "Commit review".into(),
        width: 1100,
        height: 800,
        text_config: TextConfig {
            fonts,
            text_cache_capacity: 1 << 20,
            layout_cache_capacity: 2 << 20,
        },
        text: engine,
        graphics: Box::new(gpu::Backend::new(gpu::Config::default())),
    })
    .map_err(|e| e.to_string())
}

/// A regular and a semibold face for each of the two fonts, from the system.
fn fonts(engine: &mut blit_text_cosmic::Backend) -> Result<Vec<FontFamily>, String> {
    let mut fonts = Vec::new();
    for (id, families) in [(theme::UI_FONT, MONO), (theme::MONO, MONO)] {
        let (family, regular) = families
            .iter()
            .find_map(|family| Some((*family, system_font(engine, family, 400)?)))
            .ok_or_else(|| format!("no font found among {}", families.join(", ")))?;
        let bold = system_font(engine, family, 600).unwrap_or(regular);
        let mut family = vec![engine.font_face(regular).ok_or("font disappeared")?.data.clone()];
        if bold != regular {
            family.push(engine.font_face(bold).ok_or("font disappeared")?.data.clone());
        }
        fonts.push(FontFamily { id, fonts: family });
    }
    Ok(fonts)
}

fn system_font(engine: &mut blit_text_cosmic::Backend, family: &str, weight: u16) -> Option<FontFaceId> {
    engine.system_font(SystemFontRequest { family, weight, stretch: 100, style: FontStyle::Normal }).ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Details,
    Help,
    Comments,
    Feedback,
}

impl Panel {
    fn trigger(self) -> WidgetId {
        WidgetId::new(match self {
            Self::Details => "context toggle",
            Self::Help => "help",
            Self::Comments => "comments toggle",
            Self::Feedback => "notes toggle",
        })
    }
}

enum Action {
    Deny,
    Accept,
}

pub struct App {
    output: Output,
    context: Result<Context, String>,
    review: review::Review,
    panel: Option<Panel>,
    panel_scroll: [scroll_area::State; 4],
    performance_open: bool,
    notes: String,
    notes_state: text_input::State,
    performance: performance::State,
}

impl Application for App {
    type Input = ();

    fn new(_: EventLoopProxy<()>, _: Root<Self>, _: &mut GuiContext) -> Self {
        let (command, output) = LAUNCH.with(|launch| launch.borrow_mut().take()).expect("launch set by run");
        let mut review = review::Review::default();
        review.open(command.as_deref());
        App {
            context: crate::context(command.as_deref()),
            output,
            review,
            panel: None,
            panel_scroll: Default::default(),
            performance_open: false,
            notes: String::new(),
            notes_state: text_input::State::default(),
            performance: performance::State::default(),
        }
    }

    fn input(&mut self, _: ()) {}

    fn render(&mut self, ui: Ui<'_>) {
        let notes_id = WidgetId::new("notes");
        let popup_id = WidgetId::new("panel");
        let previous = self.panel;
        let mut root = ui.layout(flex::column().padding(Sides::all(sz::LG)).gap(sz::MD));
        root.insert(Rectangle::new().background(theme::BACKGROUND));
        if self.panel.is_some_and(|panel| panel != Panel::Feedback) {
            root.focus(popup_id);
        }
        let typing = root.is_focused(notes_id) || root.is_focused(WidgetId::new("filter")) || self.review.thread.is_some();
        let mut action = None;
        match *root.input() {
            Input::Key(key) if key.pressed && key.key == Key::Escape && self.panel.is_some() => self.panel = None,
            Input::Key(key)
                if key.pressed
                    && key.key == Key::Enter
                    && (key.modifiers.control() || key.modifiers.super_key())
                    && !typing
                    && self.panel.is_none() =>
            {
                action = Some(Action::Accept)
            }
            Input::Text(key) => {
                let panel = match key {
                    '?' => Some(Panel::Help),
                    'c' => Some(Panel::Details),
                    'n' => Some(Panel::Feedback),
                    _ => None,
                };
                if let Some(panel) = panel {
                    if (!typing && self.panel.is_none()) || (self.panel == Some(panel) && panel != Panel::Feedback) {
                        self.panel = (self.panel != Some(panel)).then_some(panel);
                    }
                } else if key == 'p' && !typing && self.panel.is_none() {
                    self.performance_open = !self.performance_open;
                    root.request_frame();
                }
            }
            _ => {}
        }
        root.child().item(flex::item()).build(|ui: Ui<'_>| {
            let mut bar = ui.layout(flex::row().gap(sz::LG).align(Align::Center));
            bar.child().item(flex::item()).insert(text("REVIEW", theme::bold(sz::TEXT_SMALL), theme::ACCENT));
            let repo = self.context.as_ref().map_or("Changes", |context| context.repo.trim().rsplit('/').next().unwrap_or(&context.repo));
            bar.child().item(flex::item().grow()).insert(text(repo, theme::interface(sz::TEXT_SMALL), theme::MUTED));
            let (viewed, total) = self.review.viewed();
            bar.child().item(flex::item()).insert(text(&format!("{viewed} / {total} reviewed"), theme::mono(sz::TEXT_SMALL), theme::MUTED));
            for (panel, label) in [(Panel::Details, "Details"), (Panel::Help, "?")] {
                if bar.child().item(flex::item()).build(Button::new(panel.trigger(), label).look(if self.panel == Some(panel) {
                    Look::Selected
                } else {
                    Look::Quiet
                })) {
                    self.panel = (self.panel != Some(panel)).then_some(panel);
                }
            }
        });
        if let Err(error) = &self.context {
            root.child().item(flex::item()).insert(widgets::wrapped(error, theme::interface(sz::TEXT_SMALL), theme::DANGER));
        }
        let mut consumed = previous.is_some() || self.panel.is_some();
        let user = self.context.as_ref().map_or("You", |context| context.user.as_str());
        root.child().item(flex::item().grow()).build(|ui: Ui<'_>| review::build(ui, &mut self.review, user, &mut consumed));

        root.child().item(flex::item()).build(|ui: Ui<'_>| {
            let mut bar = ui.layout(flex::row().gap(sz::MD).align(Align::Center));
            let comments = format!("Comments {}", self.review.pending());
            for (panel, label) in [(Panel::Feedback, "Feedback"), (Panel::Comments, comments.as_str())] {
                let mut button =
                    Button::new(panel.trigger(), label).look(if self.panel == Some(panel) { Look::Selected } else { Look::Plain });
                if panel == Panel::Feedback {
                    button = button.shortcut("n");
                }
                if bar.child().item(flex::item()).build(button) {
                    self.panel = (self.panel != Some(panel)).then_some(panel);
                }
            }
            bar.child().item(flex::item().grow()).build(());
            if self.performance_open {
                bar.child().item(flex::item()).build(
                    Performance::new(&mut self.performance)
                        .popover(
                            popover::Config::new()
                                .target_anchor(Anchor::TopLeft)
                                .child_anchor(Anchor::BottomLeft)
                                .offset(Point::new(0.0, -sz::MD)),
                        )
                        .text_style(theme::mono(sz::CODE))
                        .background(theme::SURFACE)
                        .graph_background(theme::BACKGROUND)
                        .color(theme::TEXT)
                        .muted_color(theme::MUTED)
                        .accent(theme::ACCENT),
                );
            }
            if bar.child().item(flex::item()).build(Button::new(WidgetId::new("deny"), "Request changes")) {
                action = Some(Action::Deny);
            }
            if bar.child().item(flex::item()).build(Button::new(WidgetId::new("accept"), "Approve").shortcut(ACCEPT_SHORTCUT).look(Look::Primary))
            {
                action = Some(Action::Accept);
            }
        });
        if let Some(panel) = self.panel {
            let trigger = panel.trigger();
            let (title, width, above) = match panel {
                Panel::Details => ("DETAILS", sz::CONTEXT_WIDTH, false),
                Panel::Help => ("KEYBOARD CONTROLS", sz::HELP_WIDTH, false),
                Panel::Comments => ("COMMENTS", sz::HELP_WIDTH, true),
                Panel::Feedback => ("FEEDBACK FOR THE AGENT", sz::POPOVER_WIDTH, true),
            };
            if let Some(target) = root.geometry(trigger) {
                let screen = root.screen();
                let width = width.min(if above { screen.width - target.x - sz::LG } else { target.x + target.width - sz::LG }).max(0.0);
                let height = (if above { target.y } else { screen.height - target.y - target.height } - sz::LG - sz::MD).max(0.0);
                let placement = if above {
                    Absolute::attach(Anchor::TopLeft, Anchor::BottomLeft).offset(0.0, -sz::MD)
                } else {
                    Absolute::attach(Anchor::BottomRight, Anchor::TopRight).offset(0.0, sz::MD)
                };
                let mut dismiss = matches!(*root.input(), Input::PointerDown { position, button: PointerButton::Primary, .. }
                    if root.geometry(popup_id.child("done")).is_some_and(|area| area.contains(position))
                        || (root.geometry(popup_id).is_some_and(|area| !area.contains(position)) && !target.contains(position)));
                root.absolute(placement.relative_to(trigger).width(Sizing::fixed(width))).z_index(21).build(|mut ui: Ui<'_>| {
                    ui.interact(popup_id, Sense::CLICK);
                    let mut card = ui.widget_id(popup_id).layout(flex::column().padding(Sides::all(sz::LG)).gap(sz::MD));
                    card.insert(panel_surface(theme::SURFACE));
                    card.child().item(flex::item()).insert(text(title, theme::bold(sz::TEXT_SMALL), theme::MUTED));
                    let body_height = (height - sz::LG * 2.0 - sz::MD * 2.0 - sz::LINE - sz::XXXL).max(0.0);
                    let body_height = if panel == Panel::Feedback { body_height.min(sz::POPOVER_HEIGHT) } else { body_height };
                    card.child().item(flex::item().height(Sizing::fit_range(0.0, body_height))).build(
                        widgets::scroll_area(&mut self.panel_scroll[panel as usize], |ui: Ui<'_>| {
                            match panel {
                                Panel::Details => {
                                    if let Some(line) = summary::build(ui, &self.context) {
                                        if !self.notes.is_empty() && !self.notes.ends_with('\n') {
                                            self.notes.push('\n');
                                        }
                                        self.notes.push_str(&line);
                                        self.notes.push('\n');
                                        self.notes_state.cursor = self.notes.len();
                                        self.notes_state.anchor = self.notes.len();
                                        self.panel = Some(Panel::Feedback);
                                    }
                                }
                                Panel::Feedback => {
                                    dismiss |= ui.build(TextArea {
                                        state: &mut self.notes_state,
                                        id: notes_id,
                                        value: &mut self.notes,
                                        placeholder: "Feedback, including commit-message changes…",
                                        rows: 5,
                                    });
                                }
                                Panel::Comments => {
                                    let mut list = ui.layout(flex::column().gap(sz::MD));
                                    if let Some(Ok(files)) = &self.review.files {
                                        for (index, file) in files.iter().enumerate().filter(|(_, file)| !file.comments.is_empty()) {
                                            list.child().item(flex::item()).insert(widgets::wrapped(
                                                &file.display_path,
                                                theme::bold(sz::TEXT_SMALL),
                                                theme::ACCENT,
                                            ));
                                            for comment in &file.comments {
                                                list.child().item(flex::item()).build(|mut ui: Ui<'_>| {
                                                    let id = comment.id.child("summary");
                                                    let interaction = ui.interact(id, Sense::CLICK);
                                                    if interaction.activated {
                                                        self.review.reveal_comment = Some((index, comment.anchor));
                                                        dismiss = true;
                                                    }
                                                    let mut entry =
                                                        ui.widget_id(id).layout(flex::column().padding(Sides::all(sz::MD)).gap(sz::XS));
                                                    entry.insert(panel_surface(if interaction.hovered {
                                                        theme::RAISED
                                                    } else {
                                                        theme::BACKGROUND
                                                    }));
                                                    let location = crate::text::location(&file.diff, comment.anchor);
                                                    let location =
                                                        location.strip_prefix(&file.diff.path).unwrap_or(&location).trim_start_matches(':');
                                                    entry.child().item(flex::item()).insert(text(
                                                        location,
                                                        theme::mono(sz::TEXT_SMALL),
                                                        theme::MUTED,
                                                    ));
                                                    entry.child().item(flex::item().height(Sizing::fit_range(0.0, sz::LINE * 2.0))).insert(
                                                        widgets::wrapped(
                                                            comment.text.lines().next().unwrap_or(""),
                                                            theme::interface(sz::TEXT_BODY),
                                                            theme::TEXT,
                                                        ),
                                                    );
                                                });
                                            }
                                        }
                                    }
                                    if self.review.pending() == 0 {
                                        list.child().item(flex::item()).insert(text(
                                            "No comments yet",
                                            theme::interface(sz::TEXT_BODY),
                                            theme::MUTED,
                                        ));
                                    }
                                }
                                Panel::Help => {
                                    let mut list = ui.layout(flex::column().gap(sz::MD));
                                    for (section, bindings) in [
                                        (
                                            "NAVIGATE",
                                            &[
                                                ("j / k", "Next / previous file"),
                                                ("v", "Mark reviewed and advance"),
                                                ("b", "Show / hide files"),
                                                ("/", "Filter files"),
                                            ][..],
                                        ),
                                        (
                                            "REVIEW",
                                            &[
                                                ("s", "Split / unified diff"),
                                                ("c", "Review details"),
                                                ("n", "Feedback"),
                                                ("Ctrl+Enter", "Close comment / approve review"),
                                            ][..],
                                        ),
                                        (
                                            "GENERAL",
                                            &[("Esc", "Dismiss editor or panel"), ("?", "Keyboard controls"), ("p", "Performance monitor")]
                                                [..],
                                        ),
                                    ] {
                                        list.child().item(flex::item()).insert(text(section, theme::bold(sz::TEXT_LABEL), theme::MUTED));
                                        for &(key, label) in bindings {
                                            list.child().item(flex::item()).build(|ui: Ui<'_>| {
                                                let mut line = ui.layout(flex::row().gap(sz::LG));
                                                line.child().item(flex::item().grow()).insert(text(
                                                    label,
                                                    theme::interface(sz::TEXT_SMALL),
                                                    theme::TEXT,
                                                ));
                                                line.child().item(flex::item()).insert(text(key, theme::mono(sz::TEXT_SMALL), theme::ACCENT));
                                            });
                                        }
                                    }
                                    list.child().item(flex::item()).insert(widgets::wrapped(
                                        "Drag a line’s + to comment on a range. Navigation shortcuts pause while typing.",
                                        theme::interface(sz::TEXT_SMALL),
                                        theme::MUTED,
                                    ));
                                }
                            }
                        }),
                    );
                    card.child().item(flex::item()).build(|ui: Ui<'_>| {
                        let mut actions = ui.layout(flex::row().justify(Justify::End));
                        dismiss |= actions
                            .child().item(flex::item())
                            .build(Button::new(popup_id.child("done"), "Done").style(theme::interface(sz::TEXT_SMALL)));
                    });
                });
                if dismiss {
                    self.panel = None;
                }
            } else {
                root.request_frame();
            }
            action = None;
        }
        if self.panel != previous {
            root.clear_focus();
            if self.panel.is_none() {
                if let (Some(thread), Some(Ok(files))) = (self.review.thread, &mut self.review.files) {
                    if let Some(comment) = files[thread.file].comments.iter_mut().find(|comment| comment.at(thread.line)) {
                        comment.focus = true;
                    }
                }
            }
            root.request_frame();
        }
        if self.panel == Some(Panel::Feedback) && !root.is_focused(notes_id) {
            root.focus(notes_id);
            root.request_frame();
        }
        match action {
            Some(Action::Deny) => self.decide(false),
            Some(Action::Accept) => self.decide(true),
            None => {}
        }
    }
}

impl App {
    fn decide(&self, accept: bool) -> ! {
        let text = crate::text::decision(accept, &self.notes, &self.review.comment_texts());
        match self.review.reviews() {
            Some((files, reviews)) => crate::decide(self.output, accept, &text, Some((&files, reviews))),
            None => crate::decide(self.output, accept, &text, None),
        }
    }
}
