//! The diff review, in the style of GitHub's "Files changed": a file tree
//! with a filter beside the files, each with its header, comments and diff.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use blit::{Input, PointerButton, Sides, Sizing, WidgetId};
use blit_desktop::atom::Rectangle;
use blit_desktop::layout::{flex, single, Align};
use blit_desktop::style::{Border, BorderRadius};
use blit_desktop::text::TextStyle;
use blit_desktop::widget::{scroll, text_input, TextInput};
use blit_desktop::{BoundsClip, Ui};

use crate::diff::{FileDiff, Kind, Status};
use crate::message::Scope;
use crate::state::{CommentAt, FileReview, Restored};
use crate::text::{self, Anchor};

use super::lines;
use super::theme;
use super::widgets::{self, panel, Button, Checkbox, Look, ScrollArea, Tag};

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
    /// Pending comments, sent with the decision.
    pub comments: Vec<Comment>,
    /// Comments of an earlier attempt whose lines changed: not sent unless reopened.
    pub outdated: Vec<Outdated>,
}

pub struct Comment {
    pub anchor: Anchor,
    pub text: String,
    /// Carried over from an earlier attempt.
    pub earlier: bool,
}

pub struct Outdated {
    pub quote: Vec<String>,
    pub text: String,
}

/// The comment box being written; one at a time.
pub struct Form {
    pub file: usize,
    pub anchor: Anchor,
    pub text: String,
    pub state: text_input::State,
    /// The index of the comment being edited, in its file.
    pub editing: Option<usize>,
    /// Takes the keyboard focus once built.
    pub focus: bool,
}

impl Form {
    pub fn new(file: usize, anchor: Anchor, text: String, editing: Option<usize>) -> Self {
        let end = text.len();
        let state = text_input::State { cursor: end, anchor: end, offset_x: 0.0 };
        Form { file, anchor, text, state, editing, focus: true }
    }
}

/// Lines selected by dragging a "+" button, flat indices in one file.
#[derive(Clone, Copy)]
pub struct Drag {
    pub file: usize,
    pub start: usize,
    pub end: usize,
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
        let mut file = File {
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
                Restored::Lines { start, end, text } => {
                    file.comments.push(Comment { anchor: Anchor::Lines { start, end }, text, earlier: true })
                }
                Restored::File { text } => file.comments.push(Comment { anchor: Anchor::File, text, earlier: true }),
                Restored::Outdated { quote, text } => file.outdated.push(Outdated { quote, text }),
            }
        }
        file
    }
}

#[derive(Default)]
pub struct Review {
    /// Read from git when the reviewer first opens the review.
    files: Option<Result<Vec<File>, String>>,
    tree: scroll::State,
    entries: Tree,
    list: scroll::State,
    filter: String,
    filter_state: text_input::State,
    closed_dirs: HashSet<String>,
    drag: Option<Drag>,
    form: Option<Form>,
    /// A file picked in the tree, scrolled to once laid out.
    reveal: Option<usize>,
}

impl Review {
    pub fn open(&mut self, command: Option<&str>) {
        if self.files.is_none() {
            self.files = Some(crate::changes(command).map(|changes| changes.into_iter().map(File::from).collect()));
            self.entries = Tree::new(self.loaded().iter().map(|file| file.diff.path.as_str()));
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
            .flat_map(|file| file.comments.iter().map(|comment| text::comment(&file.diff, comment.anchor, &comment.text)))
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
    let Review { files, tree, entries, list, filter, filter_state, closed_dirs, drag, form, reveal } = review;
    let mut row = ui.layout(flex::row().gap(16.0));
    let files = match files {
        Some(Ok(files)) => files,
        Some(Err(error)) => {
            let message = format!("git error: {error}");
            let shown = widgets::wrapped(&message, theme::mono(theme::CODE), theme::DANGER);
            row.child(flex::item().width(Sizing::grow())).insert(shown);
            return;
        }
        None => return,
    };
    // A drag over the "+" buttons ends where the pointer is released.
    if let (Some(selected), Input::PointerUp { button: PointerButton::Primary, .. }) = (*drag, *row.input()) {
        *drag = None;
        let anchor = Anchor::Lines { start: selected.start.min(selected.end), end: selected.start.max(selected.end) };
        *form = Some(Form::new(selected.file, anchor, String::new(), None));
    }
    let mut picked = None;
    row.child(flex::item().width(Sizing::fixed(240.0)).height(Sizing::grow())).build(|ui: Ui<'_>| {
        let mut pane = ui.layout(flex::column().gap(8.0));
        pane.child(flex::item()).build(|ui: Ui<'_>| {
            let mut field = ui.layout(single::layout().padding(Sides::xy(10.0, 6.0)));
            field.insert(panel(theme::SURFACE));
            let input = TextInput::new(filter_state, WidgetId::new("filter"), filter)
                .style(theme::sans(13.0))
                .color(theme::TEXT)
                .placeholder("Filter files...")
                .placeholder_color(theme::MUTED)
                .selection_background(theme::SELECTED)
                .cursor_background(theme::TEXT);
            field.child(single::item().width(Sizing::grow())).build(input);
        });
        entries.update(files.iter().map(|file| file.diff.path.as_str()), filter, closed_dirs);
        pane.child(flex::item().grow()).build(ScrollArea::new(tree, BoundsClip).build(|ui: Ui<'_>| {
            let mut column = ui.layout(flex::column().gap(1.0));
            for &position in &entries.visible {
                let entry = &entries.all[position];
                let label = &mut entries.label;
                label.clear();
                let id = match entry {
                    Entry::Dir { path, name, depth, .. } => {
                        let chevron = if closed_dirs.contains(path) { "▸" } else { "▾" };
                        let _ = write!(label, "{:width$}{chevron}  {name}", "", width = depth * 4);
                        WidgetId::new(("tree dir", path))
                    }
                    Entry::File { index, name, depth } => {
                        let tick = if files[*index].viewed { "  ✓" } else { "" };
                        let _ = write!(label, "{:width$}    {name}{tick}", "", width = depth * 4);
                        WidgetId::new(("tree file", *index))
                    }
                };
                let button = Button::new(id, label).look(Look::Quiet).style(theme::sans(13.0));
                if column.child(flex::item()).build(button) {
                    match entry {
                        Entry::Dir { path, .. } => {
                            if !closed_dirs.remove(path) {
                                closed_dirs.insert(path.clone());
                            }
                            entries.dirty = true;
                        }
                        Entry::File { index, .. } => picked = Some(*index),
                    }
                }
            }
        }));
    });

    if let Some(index) = picked {
        files[index].collapsed = false;
        *reveal = Some(index);
    }
    if let Some(index) = *reveal {
        let content = list.id.child("content");
        if let (Some(target), Some(top)) = (row.geometry(file_id(index)), row.geometry(content)) {
            list.scroll_to(target.y - top.y);
            *reveal = None;
        }
        row.request_frame();
    }

    row.child(flex::item().grow()).build(ScrollArea::new(list, BoundsClip).build(|ui: Ui<'_>| {
        let mut column = ui.layout(flex::column().gap(14.0).padding(Sides::new().right(10.0)));
        if files.is_empty() {
            let note = widgets::text("(nothing to commit in this scope)", theme::sans(theme::BODY), theme::MUTED);
            column.child(flex::item()).insert(note);
        }
        for (index, file) in files.iter_mut().enumerate() {
            if entries.shown[index] {
                column
                    .child(flex::item())
                    .widget_id(file_id(index))
                    .build(|ui: Ui<'_>| file_box(ui, index, file, drag, form, user, consumed));
            }
        }
    }));
}

fn file_id(index: usize) -> WidgetId {
    WidgetId::new(("file", index))
}

fn file_box(ui: Ui<'_>, index: usize, file: &mut File, drag: &mut Option<Drag>, form: &mut Option<Form>, user: &str, consumed: &mut bool) {
    let mut boxed = ui.layout(flex::column());
    boxed.insert(Rectangle::new().border(Border::solid(1.0, theme::BORDER)).radius(BorderRadius::uniform(theme::RADIUS)));
    boxed.child(flex::item()).build(|ui: Ui<'_>| header(ui, index, file, form));
    if !file.collapsed {
        boxed.child(flex::item()).build(|ui: Ui<'_>| lines::body(ui, index, file, drag, form, user, consumed));
    }
}

fn header(ui: Ui<'_>, index: usize, file: &mut File, form: &mut Option<Form>) {
    let mut row = ui.layout(flex::row().padding(Sides::xy(10.0, 6.0)).gap(10.0).align(Align::Center));
    let radius = if file.collapsed {
        BorderRadius::uniform(theme::RADIUS)
    } else {
        BorderRadius::new().top_left(theme::RADIUS).top_right(theme::RADIUS)
    };
    row.insert(Rectangle::new().background(theme::SURFACE).radius(radius));
    let chevron = if file.collapsed { "▸" } else { "▾" };
    if row.child(flex::item()).build(Button::new(WidgetId::new(("chevron", index)), chevron).look(Look::Quiet)) {
        file.collapsed = !file.collapsed;
    }
    let (status, color) = match file.diff.status {
        Status::Added => ("added", theme::SUCCESS),
        Status::Deleted => ("deleted", theme::DANGER),
        Status::Renamed => ("renamed", theme::PURPLE),
        Status::Modified => ("modified", theme::WARNING),
    };
    row.child(flex::item()).build(Tag { label: status, color });
    let bold = TextStyle { weight: 600, ..theme::mono(theme::CODE) };
    row.child(flex::item().width(Sizing::grow())).insert(widgets::text(&file.display_path, bold, theme::TEXT));
    row.child(flex::item()).insert(widgets::text(&file.added_label, theme::mono(theme::CODE), theme::SUCCESS));
    row.child(flex::item()).insert(widgets::text(&file.removed_label, theme::mono(theme::CODE), theme::DANGER));
    if row.child(flex::item()).build(Checkbox { id: WidgetId::new(("viewed", index)), label: "Viewed", checked: file.viewed }) {
        file.viewed = !file.viewed;
        file.collapsed = file.viewed;
    }
    let comment = Button::new(WidgetId::new(("file comment", index)), "Comment").look(Look::Quiet).style(theme::sans(theme::SMALL));
    if row.child(flex::item()).build(comment) {
        file.collapsed = false;
        *form = Some(Form::new(index, Anchor::File, String::new(), None));
    }
}

#[derive(Default)]
struct Tree {
    all: Vec<Entry>,
    visible: Vec<usize>,
    shown: Vec<bool>,
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

        fn walk(node: Node, prefix: &str, depth: usize, entries: &mut Vec<Entry>) {
            for (name, child) in node.dirs {
                let path = format!("{prefix}{name}/");
                let position = entries.len();
                entries.push(Entry::Dir { path: path.clone(), name, depth, end: 0 });
                walk(child, &path, depth + 1, entries);
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
        walk(root, "", 0, &mut all);
        Self { all, shown, dirty: true, ..Self::default() }
    }

    fn update<'a>(&mut self, paths: impl Iterator<Item = &'a str>, filter: &str, closed: &HashSet<String>) {
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
                Entry::Dir { path, end, .. } => {
                    let matches =
                        self.all[position + 1..*end].iter().any(|entry| matches!(entry, Entry::File { index, .. } if self.shown[*index]));
                    if matches {
                        self.visible.push(position);
                    }
                    if !matches || closed.contains(path) {
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
    Dir { path: String, name: String, depth: usize, end: usize },
    File { index: usize, name: String, depth: usize },
}
