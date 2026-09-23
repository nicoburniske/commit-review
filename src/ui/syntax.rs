use std::{ops::Range, path::Path, str::FromStr, sync::OnceLock};

use blit_gui::color::Color;
use syntect::{
    easy::HighlightLines,
    highlighting::{
        Color as SyntaxColor, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
    },
    parsing::SyntaxSet,
};

use crate::diff::{FileDiff, Kind};

use super::theme;

pub struct Highlight {
    pub range: Range<usize>,
    pub color: Color,
}

#[derive(Default)]
pub struct Highlights {
    spans: Vec<Highlight>,
    lines: Vec<Range<usize>>,
}

impl Highlights {
    pub fn line(&self, index: usize) -> &[Highlight] {
        self.lines
            .get(index)
            .map(|range| &self.spans[range.clone()])
            .unwrap_or_default()
    }
}

struct Assets {
    syntaxes: SyntaxSet,
    theme: Theme,
}

pub fn highlight(file: &FileDiff) -> Highlights {
    if file.binary || file.hunks.is_empty() {
        return Highlights::default();
    }
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    let assets = ASSETS.get_or_init(|| {
        let color = |color: Color| SyntaxColor {
            r: color.red,
            g: color.green,
            b: color.blue,
            a: color.alpha,
        };
        let item = |scope, foreground| ThemeItem {
            scope: ScopeSelectors::from_str(scope).expect("valid syntax scope"),
            style: StyleModifier {
                foreground: Some(color(foreground)),
                ..StyleModifier::default()
            },
        };
        Assets {
            syntaxes: SyntaxSet::load_defaults_nonewlines(),
            theme: Theme {
                settings: ThemeSettings {
                    foreground: Some(color(theme::TEXT)),
                    background: Some(color(theme::BACKGROUND)),
                    ..ThemeSettings::default()
                },
                scopes: vec![
                    item("comment", theme::MUTED),
                    item("string", theme::SUCCESS),
                    item(
                        "constant.numeric, constant.language, constant.character",
                        Color::from_rgba8(143, 187, 194, 255),
                    ),
                    item("keyword, storage", theme::ACCENT_HOVER),
                    item("entity.name.function, support.function", theme::ACCENT),
                    item(
                        "entity.name.type, entity.name.class, support.type, support.class, entity.name.tag",
                        theme::PURPLE,
                    ),
                    item("entity.other.attribute-name", theme::ACCENT),
                ],
                ..Theme::default()
            },
        }
    });
    let path = Path::new(&file.path);
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or("");
    let Some(syntax) = assets
        .syntaxes
        .find_syntax_by_extension(name)
        .or_else(|| assets.syntaxes.find_syntax_by_extension(extension))
    else {
        return Highlights::default();
    };
    let mut result = Highlights {
        spans: Vec::new(),
        lines: Vec::with_capacity(file.lines().count()),
    };
    for hunk in &file.hunks {
        let mut old = HighlightLines::new(syntax, &assets.theme);
        let mut new = HighlightLines::new(syntax, &assets.theme);
        for line in &hunk.lines {
            let start = result.spans.len();
            let highlighter = if line.kind == Kind::Del { &mut old } else { &mut new };
            if let Ok(regions) = highlighter.highlight_line(&line.text, &assets.syntaxes) {
                let mut offset = 0;
                for (style, text) in regions {
                    let end = offset + text.len();
                    result.spans.push(Highlight {
                        range: offset..end,
                        color: Color::from_rgba8(
                            style.foreground.r,
                            style.foreground.g,
                            style.foreground.b,
                            style.foreground.a,
                        ),
                    });
                    offset = end;
                }
            }
            if line.kind == Kind::Context {
                let _ = old.highlight_line(&line.text, &assets.syntaxes);
            }
            result.lines.push(start..result.spans.len());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_supported_files() {
        let files = crate::diff::parse(
            "diff --git a/main.rs b/main.rs\n--- a/main.rs\n+++ b/main.rs\n@@ -1 +1 @@\n-fn old() {}\n+fn new() {}\n",
        );
        let highlighted = highlight(&files[0]);
        assert_eq!(highlighted.lines.len(), 2);
        assert!((0..2).all(|line| highlighted.line(line).len() > 1));
    }
}
