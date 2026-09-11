//! file navigation, diff controls and floating comments

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use blit::{Absolute, Anchor as Placement, Easing, Input, Key, NodeTarget, Sense, Sides, Sizing, Transition, WidgetId};
use blit_desktop::atom::Rectangle;
use blit_desktop::layout::{flex, single, Align, Justify};
use blit_desktop::text::TextStyle;
use blit_desktop::widget::{scroll, text_input, TextInput};
use blit_desktop::{BoundsClip, Ui};

use crate::diff::{FileDiff, Kind, Status};
use crate::message::Scope;
use crate::state::{CommentAt, FileReview, Restored};
use crate::text::{self, Anchor};

use super::theme::{self, sz};
use super::widgets::{self, panel, Button, Checkbox, Look, ScrollArea, Tag};
use super::{diff_view, lines};

pub fn scope_label(scope: Scope) -> &'static str {
    match scope {
        Scope::Staged => "Staged changes only: plain git commit",
        Scope::Tracked => "Tracked files as they are: git commit -a",
        Scope::Worktree => "Working tree, untracked files included: git add runs first",
    }
}

pub struct File {
    pub diff: FileDiff,
    pub display_path: String,
    pub added_label: String,
    pub removed_label: String,
    pub viewed: bool,
    pub collapsed: bool,
    pub source: Option<Result<Vec<Vec<crate::diff::Line>>, String>>,
    pub context: Vec<diff_view::Context>,
    /// Pending comments, sent with the decision.
    pub comments: Vec<Comment>,
    /// Comments of an earlier attempt whose lines changed: not sent unless reopened.
    pub outdated: Vec<Outdated>,
}

pub struct Comment {
    pub id: WidgetId,
    pub state: text_input::State,
    pub focus: bool,
    pub anchor: Anchor,
    pub text: String,
    /// Carried over from an earlier attempt.
    pub earlier: bool,
}

pub struct Outdated {
    pub quote: Vec<String>,
    pub text: String,
}

impl Comment {
    pub fn new(anchor: Anchor, text: String, earlier: bool) -> Self {
        let end = text.len();
        Self {
            id: WidgetId::unique(),
            anchor,
            text,
            earlier,
            state: text_input::State { cursor: end, anchor: end, offset_x: 0.0 },
            focus: true,
        }
    }

    pub fn at(&self, line: Option<usize>) -> bool {
        match (self.anchor, line) {
            (Anchor::File, None) => true,
            (Anchor::Lines { start, end }, Some(line)) => (start..=end).contains(&line),
            _ => false,
        }
    }
}

/// Lines selected by dragging a "+" button, flat indices in one file.
#[derive(Clone, Copy)]
pub struct Drag {
    pub file: usize,
    pub start: usize,
    pub end: usize,
    pub side: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Thread {
    pub file: usize,
    pub line: Option<usize>,
    pub button: Option<usize>,
}

impl From<crate::Change> for File {
    fn from(change: crate::Change) -> Self {
        let display_path = match &change.diff.old_path {
            Some(old) => format!("{old} -> {}", change.diff.path),
            None => change.diff.path.clone(),
        };
        let mut added = 0;
        let mut removed = 0;
        for line in change.diff.lines() {
            match line.kind {
                Kind::Add => added += 1,
                Kind::Del => removed += 1,
                Kind::Context => {}
            }
        }
        let mut old = 1;
        let context = change
            .diff
            .hunks
            .iter()
            .map(|hunk| {
                let total = Some((hunk.old.start - old) as usize);
                old = hunk.old.end;
                diff_view::Context { total, ..Default::default() }
            })
            .chain(std::iter::once(Default::default()))
            .collect();
        let mut file = File {
            context,
            source: None,
            collapsed: change.viewed,
            viewed: change.viewed,
            diff: change.diff,
            display_path,
            added_label: format!("+{added}"),
            removed_label: format!("-{removed}"),
            comments: Vec::new(),
            outdated: Vec::new(),
        };
        for restored in change.restored {
            match restored {
                Restored::Lines { start, end, text } => file.comments.push(Comment::new(Anchor::Lines { start, end }, text, true)),
                Restored::File { text } => file.comments.push(Comment::new(Anchor::File, text, true)),
                Restored::Outdated { quote, text } => file.outdated.push(Outdated { quote, text }),
            }
        }
        file
    }
}

#[derive(Default)]
pub struct Review {
    /// Read from git when the reviewer first opens the review.
    pub files: Option<Result<Vec<File>, String>>,
    pub tree: scroll::State,
    pub tree_width: Option<f32>,
    pub tree_hidden: bool,
    pub selected: usize,
    pub split: bool,
    pub entries: Tree,
    pub list: diff_view::State,
    pub thread: Option<Thread>,
    pub reveal_comment: Option<(usize, Anchor)>,
    pub popup: scroll::State,
    pub filter: String,
    pub filter_state: text_input::State,
    pub drag: Option<Drag>,
}

impl Review {
    pub fn open(&mut self, command: Option<&str>) {
        if self.files.is_none() {
            self.files = Some(crate::changes(command).map(|changes| changes.into_iter().map(File::from).collect()));
            self.entries = Tree::new(self.loaded().iter().map(|file| file.diff.path.as_str()));
            if let Some(Ok(files)) = &self.files {
                self.entries.update(files.iter().map(|file| file.diff.path.as_str()), &self.filter);
                self.list.rows_dirty = true;
            }
        }
    }

    fn loaded(&self) -> &[File] {
        match &self.files {
            Some(Ok(files)) => files,
            _ => &[],
        }
    }

    pub fn pending(&self) -> usize {
        self.loaded().iter().map(|file| file.comments.len()).sum()
    }

    pub fn viewed(&self) -> (usize, usize) {
        let files = self.loaded();
        (files.iter().filter(|file| file.viewed).count(), files.len())
    }

    /// The pending comments as the agent reads them.
    pub fn comment_texts(&self) -> Vec<String> {
        self.loaded()
            .iter()
            .flat_map(|file| {
                file.comments
                    .iter()
                    .filter(|comment| !comment.text.trim().is_empty())
                    .map(|comment| text::comment(&file.diff, comment.anchor, &comment.text))
            })
            .collect()
    }

    /// What to keep for the next attempt; nothing when no diff was shown.
    pub fn reviews(&self) -> Option<(Vec<FileDiff>, Vec<FileReview>)> {
        let files = self.loaded();
        if files.is_empty() {
            return None;
        }
        let reviews = files
            .iter()
            .map(|file| FileReview {
                path: file.diff.path.clone(),
                viewed: file.viewed,
                comments: file
                    .comments
                    .iter()
                    .filter(|comment| !comment.text.trim().is_empty())
                    .map(|comment| {
                        let (start, end) = match comment.anchor {
                            Anchor::File => (None, None),
                            Anchor::Lines { start, end } => (Some(start), Some(end)),
                        };
                        CommentAt { start, end, text: comment.text.clone() }
                    })
                    .collect(),
            })
            .collect();
        Some((files.iter().map(|file| file.diff.clone()).collect(), reviews))
    }
}

pub fn build(ui: Ui<'_>, review: &mut Review, user: &str, consumed: &mut bool) {
    let Review {
        files, tree, tree_width, tree_hidden, selected, split, entries, list, filter, filter_state, thread, reveal_comment, ..
    } = review;
    let mut column = ui.layout(flex::column().gap(sz::MD));
    let files = match files {
        Some(Ok(files)) => files,
        Some(Err(error)) => {
            let message = format!("git error: {error}");
            let shown = widgets::wrapped(&message, theme::mono(sz::CODE), theme::DANGER);
            column.child(flex::item().width(Sizing::grow())).insert(shown);
            return;
        }
        None => return,
    };
    let mut picked = None;
    let mut navigate = None;
    let previous_thread = *thread;
    let reveal = reveal_comment.is_some();
    if let Some((index, anchor)) = *reveal_comment {
        filter.clear();
        entries.update(files.iter().map(|file| file.diff.path.as_str()), filter);
        files[index].collapsed = false;
        list.rows_dirty = true;
        *selected = index;
        let line = match anchor {
            Anchor::File => None,
            Anchor::Lines { end, .. } => Some(end),
        };
        *thread = Some(Thread { file: index, line, button: line });
    }
    let typing = column.is_focused(WidgetId::new("notes")) || column.is_focused(WidgetId::new("filter")) || thread.is_some();
    let mut focus_filter = false;
    if let Input::Text(key) = *column.input() {
        if !typing && !*consumed {
            match key {
                'b' => {
                    *tree_hidden = !*tree_hidden;
                    column.request_frame();
                }
                '/' => {
                    *tree_hidden = false;
                    focus_filter = true;
                }
                's' => {
                    *split = !*split;
                    list.rows_dirty = true;
                    column.request_frame();
                }
                'j' | 'k' | 'v' => {
                    if key == 'v' {
                        if let Some(file) = files.get_mut(*selected) {
                            file.viewed = true;
                            file.collapsed = true;
                            list.rows_dirty = true;
                        }
                    }
                    navigate = Some(key == 'k');
                    column.request_frame();
                }
                _ => {}
            }
        }
    }
    if matches!(column.input(), Input::Key(key) if key.pressed && key.key == Key::Escape) && column.is_focused(WidgetId::new("filter")) {
        column.clear_focus();
        *consumed = true;
    }
    column.child(flex::item()).build(|ui: Ui<'_>| {
        let mut bar = ui.layout(flex::row().gap(sz::MD).align(Align::Center));
        let toggle = if *tree_hidden { "▸ Files" } else { "▾ Files" };
        if bar.child(flex::item()).build(Button::new(WidgetId::new("toggle files"), toggle).shortcut("b").look(Look::Quiet)) {
            *tree_hidden = !*tree_hidden;
            bar.request_frame();
        }
        let path = files.get(*selected).map_or("No changes", |file| file.display_path.as_str());
        bar.child(flex::item().grow()).insert(widgets::text(path, theme::mono(sz::CODE), theme::TEXT));
        for (label, backwards) in [("↑", true), ("↓", false)] {
            if bar.child(flex::item()).build(Button::new(WidgetId::new(("navigate file", backwards)), label).look(Look::Quiet)) {
                navigate = Some(backwards);
                bar.request_frame();
            }
        }
        for (label, value) in [("Unified", false), ("Split", true)] {
            if bar.child(flex::item()).build(Button::new(WidgetId::new(("diff mode", value)), label).look(if *split == value {
                Look::Selected
            } else {
                Look::Quiet
            })) && *split != value
            {
                *split = value;
                list.rows_dirty = true;
                bar.request_frame();
            }
        }
    });
    if let Some(backwards) = navigate {
        picked = if backwards {
            (0..*selected).rev().find(|&i| entries.shown[i])
        } else {
            (*selected + 1..files.len()).find(|&i| entries.shown[i])
        };
    }
    let mut row = column.child(flex::item().grow()).widget_id(WidgetId::new("review panes")).layout(flex::row());
    if !*consumed && thread.is_some() && matches!(row.input(), Input::Key(key) if key.pressed && key.key == Key::Escape) {
        *thread = None;
        *consumed = true;
        row.clear_focus();
        row.request_frame();
    }
    let divider_id = WidgetId::new("file pane divider");
    let divider = row.interact(divider_id, if *tree_hidden { Sense::default() } else { Sense::DRAG });
    let maximum = row.geometry(WidgetId::new("review panes")).map_or(sz::SIDEBAR, |area| area.width - sz::SIDEBAR_MIN - sz::LG).max(0.0);
    let width = (tree_width.unwrap_or(sz::SIDEBAR) + divider.drag_delta.x).clamp(sz::SIDEBAR_MIN.min(maximum), maximum);
    if divider.drag_delta.x != 0.0 {
        *tree_width = Some(width);
        row.request_frame();
    }
    let transition = Transition::new(if divider.active { Duration::ZERO } else { theme::TRANSITION }).easing(Easing::EaseOutQuad).layout();
    if !*tree_hidden {
        row.child(flex::item().width(Sizing::fixed(width)).height(Sizing::grow()))
            .widget_id(WidgetId::new("file pane"))
            .transition(transition)
            .clip(BoundsClip)
            .build(|ui: Ui<'_>| {
                let mut pane = ui.layout(flex::column().gap(sz::MD));
                let filter_changed = pane.child(flex::item()).build(|ui: Ui<'_>| {
                    let mut inset = ui.layout(single::layout().padding(Sides::new().left(sz::MD).right(sz::MD).top(sz::XS)));
                    let ui = inset.child(single::item().width(Sizing::grow()));
                    let mut field = ui.layout(single::layout().padding(Sides::xy(sz::MD, sz::SM)));
                    field.insert(panel(theme::SURFACE));
                    let input = TextInput::new(filter_state, WidgetId::new("filter"), filter)
                        .style(theme::interface(sz::TEXT_BODY))
                        .color(theme::TEXT)
                        .placeholder("Filter files...")
                        .placeholder_color(theme::MUTED)
                        .selection_background(theme::SELECTED)
                        .cursor_background(theme::TEXT);
                    field.child(single::item().width(Sizing::grow())).build(input).changed
                });
                entries.update(files.iter().map(|file| file.diff.path.as_str()), filter);
                if filter_changed {
                    list.rows_dirty = true;
                    if !entries.shown.get(*selected).copied().unwrap_or(false) {
                        picked = entries.shown.iter().position(|&shown| shown);
                    }
                }
                pane.child(flex::item().grow()).build(ScrollArea::new(tree, BoundsClip).build(|ui: Ui<'_>| {
                    let mut column = ui.layout(flex::column().gap(sz::BORDER));
                    for &position in &entries.visible {
                        let entry = &entries.all[position];
                        let label = &mut entries.label;
                        label.clear();
                        if let Entry::File { index, name, depth } = entry {
                            let file = &files[*index];
                            let id = WidgetId::new(("tree file", *index));
                            column.child(flex::item()).build(|mut ui: Ui<'_>| {
                                let interaction = ui.interact(id, Sense::CLICK);
                                let mut line = ui.widget_id(id).layout(
                                    flex::row()
                                        .padding(
                                            Sides::new().left(sz::MD + *depth as f32 * sz::LG).right(sz::SM).top(sz::SM).bottom(sz::SM),
                                        )
                                        .gap(sz::SM)
                                        .align(Align::Center),
                                );
                                line.insert(Rectangle::new().background(if *selected == *index {
                                    theme::SELECTED
                                } else if interaction.hovered {
                                    theme::RAISED
                                } else {
                                    theme::BACKGROUND
                                }));
                                line.child(flex::item()).insert(widgets::text(
                                    if file.viewed { "✓" } else { "·" },
                                    theme::mono(sz::TEXT_SMALL),
                                    if file.viewed { theme::SUCCESS } else { theme::MUTED },
                                ));
                                line.child(flex::item().grow()).insert(widgets::text(
                                    name,
                                    theme::interface(sz::TEXT_SMALL),
                                    if *selected == *index { theme::ACCENT } else { theme::TEXT },
                                ));
                                if !file.comments.is_empty() {
                                    let _ = write!(label, "{}●", file.comments.len());
                                    line.child(flex::item()).insert(widgets::text(label, theme::mono(sz::TEXT_TINY), theme::ACCENT));
                                }
                                line.child(flex::item()).insert(widgets::text(
                                    &file.added_label,
                                    theme::mono(sz::TEXT_TINY),
                                    theme::SUCCESS,
                                ));
                                line.child(flex::item()).insert(widgets::text(
                                    &file.removed_label,
                                    theme::mono(sz::TEXT_TINY),
                                    theme::DANGER,
                                ));
                                if interaction.clicked {
                                    picked = Some(*index);
                                    line.request_frame();
                                }
                            });
                            continue;
                        }
                        let Entry::Dir { name, depth, collapsed, .. } = entry else { continue };
                        let chevron = if *collapsed { "▸" } else { "▾" };
                        let _ = write!(label, "{:width$}{chevron}  {name}", "", width = depth * 2);
                        let id = WidgetId::new(("tree dir", position));
                        let button = Button::new(id, label).look(Look::Quiet).style(theme::interface(sz::TEXT_BODY));
                        if column.child(flex::item()).build(button) {
                            if let Entry::Dir { collapsed, .. } = &mut entries.all[position] {
                                *collapsed = !*collapsed;
                            }
                            entries.dirty = true;
                        }
                    }
                }));
            });
        let mut grip = row
            .child(flex::item().width(Sizing::fixed(sz::LG)).height(Sizing::grow()))
            .widget_id(divider_id)
            .layout(flex::row().align(Align::Center).justify(Justify::Center));
        grip.child(flex::item().fixed(sz::XS, sz::XXXL)).insert(
            Rectangle::new().background(if divider.active || divider.hovered { theme::ACCENT } else { theme::MUTED }),
        );
    }

    if let Some(index) = picked {
        *selected = index;
    }
    if let Some(index) = picked.filter(|&index| files[index].collapsed) {
        files[index].collapsed = false;
        list.rows_dirty = true;
    }
    if focus_filter {
        row.focus(WidgetId::new("filter"));
    }
    let anchor = row
        .child(flex::item().grow())
        .widget_id(WidgetId::new("diff pane"))
        .transition(transition)
        .build(|ui: Ui<'_>| diff_view::build(ui, review, picked));
    let Review { files, list, thread, popup, .. } = review;
    let Some(Ok(files)) = files else { return };
    if previous_thread != *thread || reveal {
        popup.scroll_to(0.0);
        if let Some(open) = thread {
            if let Some(comment) = files[open.file].comments.iter_mut().find(|comment| comment.at(open.line)) {
                comment.focus = true;
            }
        }
    }
    if let (Some(Thread { file: index, line, .. }), Some(anchor)) = (*thread, anchor) {
        let screen = row.screen();
        let bounds = row.geometry(list.list.id()).unwrap_or(screen);
        let width = sz::POPOVER_WIDTH.min((bounds.width - sz::XL).max(1.0));
        let max_height = (bounds.height - sz::XL).max(1.0);
        let height = row.geometry(popup.id.child("content")).map_or(sz::POPOVER_HEIGHT, |area| area.height).min(max_height);
        let target = row.geometry(anchor);
        let below = target.is_none_or(|target| target.y + target.height + height + sz::MD <= bounds.y + bounds.height);
        let x = target.map_or(0.0, |target| {
            target.x.clamp(bounds.x + sz::MD, (bounds.x + bounds.width - width - sz::MD).max(bounds.x + sz::MD)) - target.x
        });
        let y = target.map_or(0.0, |target| {
            let top = if below { target.y + target.height + sz::XS } else { target.y - height - sz::XS };
            top.clamp(bounds.y + sz::MD, (bounds.y + bounds.height - height - sz::MD).max(bounds.y + sz::MD)) - top
        });
        if target.is_none() || row.geometry(popup.id).is_none_or(|area| area.height != height || area.width != width) {
            row.request_frame();
        }
        let placement = Absolute::attach(
            if below { Placement::BottomLeft } else { Placement::TopLeft },
            if below { Placement::TopLeft } else { Placement::BottomLeft },
        )
        .relative_to(anchor)
        .offset(x, y + if below { sz::XS } else { -sz::XS })
        .width(Sizing::fixed(width))
        .height(Sizing::fit_range(0.0, max_height));
        let mut close = false;
        row.absolute(placement).parent(NodeTarget::Root).z_index(10).build(ScrollArea::new(popup, BoundsClip).build(|ui: Ui<'_>| {
            close = lines::popover(ui, &mut files[index], line, user, consumed);
        }));
        if close {
            *thread = None;
            row.clear_focus();
            row.request_frame();
        }
    }
    for (index, file) in files.iter_mut().enumerate() {
        file.comments
            .retain(|comment| !comment.text.trim().is_empty() || thread.is_some_and(|open| open.file == index && comment.at(open.line)));
    }
}

pub fn header(ui: Ui<'_>, index: usize, file: &File, label: &mut String) -> Option<HeaderAction> {
    let mut action = None;
    let mut row = ui.layout(flex::row().padding(Sides::xy(sz::MD, sz::SM)).gap(sz::MD).align(Align::Center));
    row.insert(Rectangle::new().background(theme::SURFACE));
    let chevron = if file.collapsed { "▸" } else { "▾" };
    if row.child(flex::item()).build(Button::new(WidgetId::new(("chevron", index)), chevron).look(Look::Quiet)) {
        action = Some(HeaderAction::Collapse);
    }
    let (status, color) = match file.diff.status {
        Status::Added => ("added", theme::SUCCESS),
        Status::Deleted => ("deleted", theme::DANGER),
        Status::Renamed => ("renamed", theme::PURPLE),
        Status::Modified => ("modified", theme::WARNING),
    };
    row.child(flex::item()).build(Tag { label: status, color });
    let bold = TextStyle { weight: 600, ..theme::mono(sz::CODE) };
    row.child(flex::item().width(Sizing::grow())).insert(widgets::text(&file.display_path, bold, theme::TEXT));
    row.child(flex::item()).insert(widgets::text(&file.added_label, theme::mono(sz::CODE), theme::SUCCESS));
    row.child(flex::item()).insert(widgets::text(&file.removed_label, theme::mono(sz::CODE), theme::DANGER));
    if row.child(flex::item()).build(Checkbox { id: WidgetId::new(("viewed", index)), label: "Reviewed", checked: file.viewed }) {
        action = Some(HeaderAction::Viewed);
    }
    let count = file.comments.iter().filter(|comment| matches!(comment.anchor, Anchor::File)).count() + file.outdated.len();
    let label = if count == 0 {
        "Comment"
    } else {
        label.clear();
        let _ = write!(label, "Comments ({count})");
        label.as_str()
    };
    let comment = Button::new(WidgetId::new(("file comment", index)), label).look(Look::Quiet).style(theme::interface(sz::TEXT_SMALL));
    if row.child(flex::item()).build(comment) {
        action = Some(HeaderAction::Comment);
    }
    if action.is_some() {
        row.request_frame();
    }
    action
}

pub enum HeaderAction {
    Collapse,
    Viewed,
    Comment,
}

#[derive(Default)]
pub struct Tree {
    all: Vec<Entry>,
    visible: Vec<usize>,
    pub shown: Vec<bool>,
    filter: String,
    label: String,
    dirty: bool,
}

impl Tree {
    fn new<'a>(paths: impl Iterator<Item = &'a str>) -> Self {
        #[derive(Default)]
        struct Node {
            dirs: BTreeMap<String, Node>,
            files: Vec<(String, usize)>,
        }

        fn walk(node: Node, depth: usize, entries: &mut Vec<Entry>) {
            for (name, child) in node.dirs {
                let position = entries.len();
                entries.push(Entry::Dir { name, depth, end: 0, collapsed: false });
                walk(child, depth + 1, entries);
                let end = entries.len();
                if let Entry::Dir { end: slot, .. } = &mut entries[position] {
                    *slot = end;
                }
            }
            let mut files = node.files;
            files.sort_unstable();
            entries.extend(files.into_iter().map(|(name, index)| Entry::File { index, name, depth }));
        }

        let mut root = Node::default();
        let mut shown = Vec::new();
        for (index, path) in paths.enumerate() {
            shown.push(true);
            let mut parts = path.split('/').peekable();
            let mut node = &mut root;
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    node.files.push((part.to_string(), index));
                } else {
                    node = node.dirs.entry(part.to_string()).or_default();
                }
            }
        }
        let mut all = Vec::new();
        walk(root, 0, &mut all);
        Self { all, shown, dirty: true, ..Self::default() }
    }

    fn update<'a>(&mut self, paths: impl Iterator<Item = &'a str>, filter: &str) {
        let filter = filter.trim();
        if self.filter != filter {
            self.filter.clear();
            self.filter.push_str(filter);
            let query = filter.to_lowercase();
            for (shown, path) in self.shown.iter_mut().zip(paths) {
                *shown = query.is_empty() || path.to_lowercase().contains(&query);
            }
            self.dirty = true;
        }
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.visible.clear();
        let mut position = 0;
        while position < self.all.len() {
            match &self.all[position] {
                Entry::Dir { collapsed, end, .. } => {
                    let matches =
                        self.all[position + 1..*end].iter().any(|entry| matches!(entry, Entry::File { index, .. } if self.shown[*index]));
                    if matches {
                        self.visible.push(position);
                    }
                    if !matches || *collapsed {
                        position = *end;
                        continue;
                    }
                }
                Entry::File { index, .. } => {
                    if self.shown[*index] {
                        self.visible.push(position);
                    }
                }
            }
            position += 1;
        }
    }
}

enum Entry {
    Dir { name: String, depth: usize, end: usize, collapsed: bool },
    File { index: usize, name: String, depth: usize },
}
