//! The Checks page: whether each task is fit to train on, from the two fixed agents
//! an environment engineer runs on it. The reference solution (`oracle`) must reach
//! full reward every time and doing nothing (`nop`) must score zero; a task that
//! fails the first is broken, and one that fails the second has a grader that passes
//! untouched work. The rule itself is `seamlessh_runs::qualify`; this draws it.

use gpui::{AnyElement, Context, Div, div, prelude::*, px};
use seamlessh_runs::qualify::{Side, TaskCheck, Verdict, qualify};

use super::screen::{body, button, cell_box, name_cell, page_head, quiet, section_head, stats, th};
use super::style::{self as kit, DIM, EMBER, INK_2, MIST, MONO, WORKING};
use super::{Page, ResearchView};

/// Oracle, no-op and verdict columns; the task takes the rest.
const W: [f32; 3] = [130., 96., 150.];

impl ResearchView {
    pub(super) fn checks_page(&self, cx: &mut Context<Self>) -> (Div, Option<Div>) {
        let runs = self.ui.runs();
        let rollouts: Vec<_> = runs
            .iter()
            .flat_map(|(_, run)| run.rollouts.iter().cloned())
            .collect();
        let mut checks = qualify(&rollouts);
        // Faults first: they are what the page is for.
        checks.sort_by_key(|c| rank(c.verdict));
        let scope = match self.ui.scope {
            Some(place) => self.place_name(place),
            None => "all folders".to_owned(),
        };
        let count_of = |v: &[Verdict]| checks.iter().filter(|c| v.contains(&c.verdict)).count();
        let to_runs = button("research-checks-to-runs", "Runs", false).on_click(cx.listener(
            |this, _, _, cx| {
                this.ui.page = Page::Runs;
                cx.notify();
            },
        ));
        let section = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(page_head(
                "Checks",
                match checks.len() {
                    1 => format!("1 task checked in {scope}"),
                    n => format!("{n} tasks checked in {scope}"),
                },
                false,
                vec![to_runs.into_any_element()],
            ))
            .child(stats(vec![
                ("Sound", count_of(&[Verdict::Sound]).to_string()),
                (
                    "Oracle fails",
                    count_of(&[Verdict::OracleFails, Verdict::Both]).to_string(),
                ),
                (
                    "No-op passes",
                    count_of(&[Verdict::NopPasses, Verdict::Both]).to_string(),
                ),
                ("Half checked", count_of(&[Verdict::Unchecked]).to_string()),
            ]));
        if checks.is_empty() {
            return (
                section.child(body(
                    "No task has an oracle or no-op run yet. Run Harbor's oracle agent \
                     (harbor run -a oracle) and a no-op agent on a task; each task then \
                     shows here with whether its solution passes and doing nothing fails.",
                    DIM,
                )),
                None,
            );
        }
        let head = div()
            .flex()
            .child(th("Task", None, true).min_w(px(200.)))
            .child(th("Oracle", Some(W[0]), false))
            .child(th("No-op", Some(W[1]), false))
            .child(th("Verdict", Some(W[2]), false));
        let rows = checks
            .iter()
            .enumerate()
            .map(|(at, check)| check_row(at, check));
        let section = section.child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(section_head(
                    "By task",
                    None,
                    Some("Faults first".to_owned()),
                ))
                .child(
                    div()
                        .id("research-checks")
                        .w_full()
                        .overflow_x_scroll()
                        .child(
                            div()
                                .min_w(px(560.))
                                .flex()
                                .flex_col()
                                .child(head)
                                .children(rows),
                        ),
                )
                .child(quiet(
                    "Oracle: rollouts at full reward of those run. No-op: rollouts above zero. \
                     The oracle must pass every time and the no-op never · , . folder · 1 2 3 page",
                )),
        );
        (section, None)
    }
}

fn check_row(at: usize, check: &TaskCheck) -> AnyElement {
    let (color, word) = verdict(check.verdict);
    div()
        .id(("research-check", at as u64))
        .flex()
        .items_center()
        .py(px(10.))
        .border_t_1()
        .border_color(kit::white(0.06))
        .child(
            cell_box(None, true)
                .min_w(px(200.))
                .child(name_cell(check.task.label.clone())),
        )
        .child(cell_box(Some(W[0]), false).child(side(check.oracle.as_ref(), Want::All)))
        .child(cell_box(Some(W[1]), false).child(side(check.nop.as_ref(), Want::None)))
        .child(cell_box(Some(W[2]), false).child(kit::status(color, word)))
        .into_any_element()
}

/// Which count of passes is the good one for a side.
#[derive(Clone, Copy)]
enum Want {
    All,
    None,
}

/// "3/3" in monospace, ember when the side is at fault, a dash when it never ran.
fn side(side: Option<&Side>, want: Want) -> Div {
    let Some(s) = side else {
        return body("–", DIM).font_family(MONO);
    };
    let fault = side_fault(s, want);
    let mut text = format!("{}/{}", s.passed, s.rollouts);
    if s.errored > 0 {
        text.push_str(&format!(" · {} err", s.errored));
    }
    body(text, if fault { EMBER } else { INK_2 })
        .font_family(MONO)
        .whitespace_nowrap()
}

fn side_fault(s: &Side, want: Want) -> bool {
    s.errored > 0
        || match want {
            Want::All => s.passed < s.rollouts,
            Want::None => s.passed > 0,
        }
}

fn verdict(v: Verdict) -> (u32, &'static str) {
    match v {
        Verdict::Sound => (WORKING, "Sound"),
        Verdict::OracleFails => (EMBER, "Oracle fails"),
        Verdict::NopPasses => (EMBER, "No-op passes"),
        Verdict::Both => (EMBER, "Both fail"),
        Verdict::Unchecked => (MIST, "Half checked"),
    }
}

fn rank(v: Verdict) -> u8 {
    match v {
        Verdict::Both => 0,
        Verdict::NopPasses => 1,
        Verdict::OracleFails => 2,
        Verdict::Unchecked => 3,
        Verdict::Sound => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faults_sort_before_half_checked_and_sound() {
        let mut v = [
            Verdict::Sound,
            Verdict::Unchecked,
            Verdict::OracleFails,
            Verdict::Both,
            Verdict::NopPasses,
        ];
        v.sort_by_key(|v| rank(*v));
        assert_eq!(
            v,
            [
                Verdict::Both,
                Verdict::NopPasses,
                Verdict::OracleFails,
                Verdict::Unchecked,
                Verdict::Sound,
            ]
        );
    }

    #[test]
    fn a_side_is_at_fault_by_what_it_should_do() {
        let s = |passed, rollouts, errored| Side {
            rollouts,
            passed,
            errored,
        };
        assert!(!side_fault(&s(3, 3, 0), Want::All));
        assert!(side_fault(&s(2, 3, 0), Want::All));
        assert!(side_fault(&s(3, 3, 1), Want::All));
        assert!(!side_fault(&s(0, 3, 0), Want::None));
        assert!(side_fault(&s(1, 3, 0), Want::None));
    }
}
