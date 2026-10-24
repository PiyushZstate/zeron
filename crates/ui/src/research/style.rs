//! The locked Research design's values and small parts, ported from Seamlessh's
//! `theme.rs` and `kit.rs` (docs/research-view.md there, owner 2026-10-24) so the
//! screen reads the same numbers as its boards. Only what the Research screen
//! draws is here; the rest of Zeron keeps its own theme.
//!
//! The rules they keep: greys carry the interface and the blue is only the primary
//! button and a selected row's tint on a panel; a status is a dot and a word, never a
//! filled pill; labels are sentence case in the quiet grey; monospace is for names,
//! numbers and code; cards are flat and never nest.

use gpui::{
    Background, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, div, hsla, prelude::*,
    px, rgb,
};

/// The panel's ground: the terminal's own tone.
pub const GROUND: u32 = 0x0a0a0a;
/// Code and tool output: a step up from the panel.
pub const INSET: u32 = 0x171717;
/// A stat cell: a step below the inset, so a row of them reads as one band.
pub const CARD: u32 = 0x121214;
pub const RAISED: u32 = 0x1c1c1c;
pub const SELECTED: u32 = 0x292929;

pub const INK: u32 = 0xededed;
pub const INK_2: u32 = 0xd8d8d8;
pub const DIM: u32 = 0x9e9e9e;
pub const SOFT: u32 = 0xc2c2c2;
pub const MIST: u32 = 0x8f8f8f;

pub const R_PANEL: f32 = 10.;
pub const R_ROW: f32 = 6.;

/// Waiting on the person.
pub const WAITING: u32 = 0xf0b100;
/// Working, and a full reward.
pub const WORKING: u32 = 0x00bc7d;
/// Failure.
pub const EMBER: u32 = 0xfb2c36;
/// An improvement against a baseline.
pub const ADDED: u32 = 0x56a482;
/// The one primary action, and a selected row's tint. White on it measures 5.2:1.
pub const ACCENT: u32 = 0x155dfc;
const ON_PLATE: u32 = 0xffffff;
/// How a plate answers the pointer.
const PLATE_HOVER: f32 = 0.85;

/// Monospace, for names, numbers and code. Zeron bundles it.
pub const MONO: &str = "Geist Mono";

const UI: f32 = 13.5;
const UI_LINE: f32 = 19.;
const META: f32 = 12.;
const META_LINE: f32 = 16.;
const CODE: f32 = 12.;
const CODE_LINE: f32 = 18.;
/// A button's height: a list row's, so a button in a row fills it.
const BUTTON_H: f32 = 28.;

/// White at `alpha`: every hover and wash is one.
pub fn white(alpha: f32) -> Hsla {
    hsla(0., 0., 1., alpha)
}

/// A selected row in a list on a panel: the accent, faint.
pub fn accent_tint() -> Hsla {
    let mut tint: Hsla = rgb(ACCENT).into();
    tint.a = 0.12;
    tint
}

pub fn row_hover() -> Hsla {
    white(0.035)
}

/// A panel's own 1px ring, so its dark corner meets the frame softly.
pub fn panel_ring() -> Hsla {
    white(0.05)
}

fn plate() -> Background {
    rgb(ACCENT).into()
}

/// Interface text, 13.5/19.
pub fn ui(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .text_size(px(UI))
        .line_height(px(UI_LINE))
        .text_color(rgb(color))
        .child(text.into())
}

/// Meta text, 12/16: the quiet words beside things.
pub fn meta(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .text_size(px(META))
        .line_height(px(META_LINE))
        .text_color(rgb(color))
        .child(text.into())
}

/// What a button is for. There is one primary action in a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The view's one action: the blue plate with white words.
    Primary,
    /// Any other action: a raised grey.
    Secondary,
}

/// A button with `words`. The caller adds its `on_click`.
pub fn button(
    id: impl Into<ElementId>,
    words: impl Into<SharedString>,
    kind: Kind,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex_none()
        .h(px(BUTTON_H))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_ROW))
        .cursor_pointer()
        .text_size(px(UI))
        .line_height(px(UI_LINE))
        .font_weight(FontWeight::MEDIUM);
    match kind {
        Kind::Primary => base
            .bg(plate())
            .hover(|style| style.opacity(PLATE_HOVER))
            .text_color(rgb(ON_PLATE))
            .child(words.into()),
        Kind::Secondary => base
            .bg(rgb(RAISED))
            .hover(|style| style.bg(rgb(SELECTED)))
            .text_color(rgb(INK_2))
            .child(words.into()),
    }
}

/// A tab in a row of them: the words, a count beside them in monospace, and a raised
/// grey behind the chosen one.
pub fn tab(
    id: impl Into<ElementId>,
    words: impl Into<SharedString>,
    count: Option<SharedString>,
    chosen: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(26.))
        .px(px(9.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(R_ROW))
        .cursor_pointer()
        .when(chosen, |tab| tab.bg(rgb(RAISED)))
        .when(!chosen, |tab| tab.hover(|style| style.bg(white(0.04))))
        .child(ui(words, if chosen { INK } else { DIM }))
        .children(count.map(|n| ui(n, DIM).font_family(MONO)))
}

/// The dot a status is drawn with: 6 px, its state's colour.
pub fn dot(color: u32) -> Div {
    div().flex_none().size(px(6.)).rounded_full().bg(rgb(color))
}

/// A status: a dot in its state's colour, then the word in interface ink. The word
/// carries the meaning; the colour only echoes it.
pub fn status(color: u32, word: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(7.))
        .child(dot(color))
        .child(ui(word, INK_2))
}

/// A stat cell: a flat block a step above the panel, its name a label and its figure
/// large in monospace.
pub fn stat(name: impl Into<SharedString>, figure: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .px(px(14.))
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .rounded(px(R_PANEL))
        .bg(rgb(CARD))
        .child(meta(name, DIM))
        .child(
            div()
                .font_family(MONO)
                .text_size(px(20.))
                .line_height(px(28.))
                .text_color(rgb(INK))
                .child(figure.into()),
        )
}

/// Code, a command or log lines in an inset: monospace on the inset grey.
pub fn code(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .w_full()
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(rgb(INSET))
        .font_family(MONO)
        .text_size(px(CODE))
        .line_height(px(CODE_LINE))
        .text_color(rgb(color))
        .child(text.into())
}
