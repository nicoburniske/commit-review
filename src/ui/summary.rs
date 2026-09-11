//! commit message findings and amend context

use blit::WidgetId;
use blit_desktop::layout::{flex, Align};
use blit_desktop::text::TextWrap;
use blit_desktop::Ui;

use crate::message::Kind;
use crate::{text, Context};

use super::marked::Marked;
use super::theme::{self, sz};
use super::widgets::{self, Button, Look};

pub fn build(ui: Ui<'_>, context: &Result<Context, String>) -> Option<String> {
    let mut column = ui.layout(flex::column().gap(sz::LG));
    let context = match context {
        Ok(context) => context,
        Err(error) => {
            column.child(flex::item()).insert(widgets::wrapped(error, theme::mono(sz::CODE), theme::DANGER));
            return None;
        }
    };
    let mut clicked = None;
    column.child(flex::item()).insert(widgets::text(
        &format!("{} · {}", context.branch.trim(), super::review::scope_label(context.scope)),
        theme::interface(sz::TEXT_SMALL),
        theme::MUTED,
    ));
    column.child(flex::item()).build(|ui: Ui<'_>| {
        let mut section = ui.layout(flex::column().gap(sz::XS));
        let heading = match &context.amend {
            Some(amend) if amend.message_kept => "COMMIT MESSAGE   kept as is",
            Some(_) => "COMMIT MESSAGE   replaces the current one",
            None => "COMMIT MESSAGE",
        };
        section.child(flex::item()).build(|ui: Ui<'_>| {
            let mut row = ui.layout(flex::row().gap(sz::MD).align(Align::Center));
            row.child(flex::item()).insert(widgets::text(heading, theme::interface(sz::TEXT_LABEL), theme::MUTED));
            if let Some(findings) = &context.findings {
                for (field, found) in [("subject", &findings.subject), ("body", &findings.body)] {
                    for kind in [Kind::NonAscii, Kind::Email, Kind::Link, Kind::CoAuthoredBy] {
                        let count = found.iter().filter(|finding| finding.kind == kind).count();
                        if count == 0 {
                            continue;
                        }
                        let badge = text::badge(field, kind, count);
                        let look = if kind == Kind::NonAscii { Look::Danger } else { Look::Warning };
                        let id = WidgetId::new(("badge", field, kind as u8));
                        if row.child(flex::item()).build(Button::new(id, &badge).look(look).style(theme::interface(sz::TEXT_LABEL))) {
                            clicked = Some(badge);
                        }
                    }
                }
            }
        });
        if let (Some(message), Some(findings)) = (&context.message, &context.findings) {
            for (value, marks, style) in
                [(&message.subject, &findings.subject, theme::bold(sz::TEXT_TITLE)), (&message.body, &findings.body, theme::mono(sz::CODE))]
            {
                if !value.is_empty() {
                    let (text, marks) = text::marked(value, marks);
                    section.child(flex::item()).insert(Marked { text: &text, marks: &marks, style, wrap: TextWrap::Word });
                }
            }
        } else {
            let note = if context.command.is_some() { "(commit message could not be read)" } else { "(manual launch, no command given)" };
            section.child(flex::item()).insert(widgets::text(note, theme::interface(sz::TEXT_BODY), theme::MUTED));
        }
    });
    if let Some(amend) = &context.amend {
        column.child(flex::item()).build(|ui: Ui<'_>| {
            let mut section = ui.layout(flex::column().gap(sz::XS));
            section.child(flex::item()).insert(widgets::text(
                &format!("ALREADY IN HEAD   {}", amend.head),
                theme::interface(sz::TEXT_LABEL),
                theme::MUTED,
            ));
            section.child(flex::item()).insert(widgets::text(&amend.stat, theme::mono(sz::CODE), theme::TEXT));
        });
    }
    clicked
}
