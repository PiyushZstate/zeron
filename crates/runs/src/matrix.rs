//! Pure aggregation of rollouts into the run matrix (docs/research-view.md).
//!
//! Rows, columns and filters are chosen axes. A missing value groups under
//! [`crate::NOT_RECORDED`]. Errors and missing rewards stay out of the mean.

use serde::{Deserialize, Serialize};

use crate::model::{Rollout, axis_text, mean};

/// One of the five matrix axes. Variant order is append-only (postcard).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Axis {
    Task,
    Env,
    Harness,
    Model,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixSpec {
    pub rows: Axis,
    pub columns: Vec<Axis>,
    pub filters: Vec<(Axis, String)>,
}

impl Default for MatrixSpec {
    fn default() -> Self {
        Self {
            rows: Axis::Task,
            columns: vec![Axis::Harness, Axis::Model],
            filters: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Matrix {
    /// Row keys, first-seen order.
    pub rows: Vec<String>,
    /// One value per column axis, first-seen order of observed combinations.
    pub columns: Vec<Vec<String>>,
    /// Only cells that have at least one rollout.
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub row: usize,
    pub column: usize,
    /// Over rollouts that [`Rollout::counts_in_mean`]; `None` when none do.
    pub mean: Option<f64>,
    pub counted: u32,
    pub rollouts: u32,
    pub errored: u32,
    /// Indices into the input slice, so a cell can open its rollouts.
    pub members: Vec<usize>,
}

impl Matrix {
    pub fn cell(&self, row: usize, column: usize) -> Option<&Cell> {
        self.cells
            .iter()
            .find(|c| c.row == row && c.column == column)
    }
}

/// Group `rollouts` by `spec`. Order of rows, columns and cells is first seen.
pub fn matrix(rollouts: &[Rollout], spec: &MatrixSpec) -> Matrix {
    let mut rows: Vec<String> = Vec::new();
    let mut columns: Vec<Vec<String>> = Vec::new();
    let mut cells: Vec<Cell> = Vec::new();
    for (i, r) in rollouts.iter().enumerate() {
        if !spec
            .filters
            .iter()
            .all(|(axis, want)| axis_value(r, *axis) == want)
        {
            continue;
        }
        let row = first_seen(&mut rows, axis_value(r, spec.rows).to_owned());
        let col_key: Vec<String> = spec
            .columns
            .iter()
            .map(|axis| axis_value(r, *axis).to_owned())
            .collect();
        let column = first_seen(&mut columns, col_key);
        let at = match cells
            .iter()
            .position(|c| c.row == row && c.column == column)
        {
            Some(at) => at,
            None => {
                cells.push(Cell {
                    row,
                    column,
                    mean: None,
                    counted: 0,
                    rollouts: 0,
                    errored: 0,
                    members: Vec::new(),
                });
                cells.len() - 1
            }
        };
        let cell = &mut cells[at];
        cell.rollouts += 1;
        if r.outcome.is_error() {
            cell.errored += 1;
        }
        cell.members.push(i);
    }
    for cell in &mut cells {
        let counted: Vec<f64> = cell
            .members
            .iter()
            .filter_map(|&i| {
                let r = rollouts.get(i)?;
                r.counts_in_mean().then_some(r.reward).flatten()
            })
            .collect();
        cell.counted = counted.len() as u32;
        cell.mean = mean(counted.into_iter());
    }
    Matrix {
        rows,
        columns,
        cells,
    }
}

fn axis_value(rollout: &Rollout, axis: Axis) -> &str {
    let raw = match axis {
        Axis::Task => Some(rollout.task.key.as_str()),
        Axis::Env => rollout.axes.env.as_deref(),
        Axis::Harness => rollout.axes.harness.as_deref(),
        Axis::Model => rollout.axes.model.as_deref(),
        Axis::Runtime => rollout.axes.runtime.as_deref(),
    };
    axis_text(raw)
}

fn first_seen<T: PartialEq>(list: &mut Vec<T>, key: T) -> usize {
    match list.iter().position(|k| *k == key) {
        Some(at) => at,
        None => {
            list.push(key);
            list.len() - 1
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::{Axes, NOT_RECORDED, Outcome, TaskId, Tokens};

    fn r(
        id: &str,
        task: &str,
        harness: Option<&str>,
        model: Option<&str>,
        runtime: Option<&str>,
        outcome: Outcome,
        reward: Option<f64>,
    ) -> Rollout {
        Rollout {
            id: id.into(),
            task: TaskId {
                key: task.into(),
                label: task.into(),
                checksum: None,
            },
            outcome,
            stop: None,
            error: None,
            rewards: vec![],
            reward,
            tokens: Tokens::default(),
            cost: None,
            duration_secs: None,
            started_at: None,
            traces: 1,
            axes: Axes {
                env: None,
                harness: harness.map(str::to_owned),
                model: model.map(str::to_owned),
                runtime: runtime.map(str::to_owned),
            },
        }
    }

    #[test]
    fn two_harnesses_one_model_are_two_columns_of_one_task() {
        let rollouts = [
            r(
                "a",
                "t",
                Some("bash"),
                Some("qwen"),
                None,
                Outcome::Completed,
                Some(1.0),
            ),
            r(
                "b",
                "t",
                Some("python"),
                Some("qwen"),
                None,
                Outcome::Completed,
                Some(0.0),
            ),
        ];
        let m = matrix(&rollouts, &MatrixSpec::default());
        assert_eq!(m.rows, ["t"]);
        assert_eq!(
            m.columns,
            [
                vec!["bash".to_owned(), "qwen".to_owned()],
                vec!["python".to_owned(), "qwen".to_owned()]
            ]
        );
        assert_eq!(m.cells.len(), 2);
        assert_eq!(m.cell(0, 0).unwrap().mean, Some(1.0));
        assert_eq!(m.cell(0, 1).unwrap().mean, Some(0.0));
    }

    #[test]
    fn errored_rollouts_do_not_move_the_mean() {
        // 1.0, 0.0 and an errored 0.9: the mean is 0.5.
        let rollouts = [
            r(
                "a",
                "t",
                Some("bash"),
                Some("qwen"),
                None,
                Outcome::Completed,
                Some(1.0),
            ),
            r(
                "b",
                "t",
                Some("bash"),
                Some("qwen"),
                None,
                Outcome::Completed,
                Some(0.0),
            ),
            r(
                "c",
                "t",
                Some("bash"),
                Some("qwen"),
                None,
                Outcome::Errored,
                Some(0.9),
            ),
        ];
        let c = matrix(&rollouts, &MatrixSpec::default())
            .cell(0, 0)
            .unwrap()
            .clone();
        assert_eq!(c.mean, Some(0.5));
        assert_eq!((c.counted, c.rollouts, c.errored), (2, 3, 1));
    }

    #[test]
    fn a_cell_of_only_errors_has_no_mean() {
        let rollouts = [r(
            "a",
            "t",
            Some("bash"),
            Some("qwen"),
            None,
            Outcome::Errored,
            Some(0.9),
        )];
        let c = matrix(&rollouts, &MatrixSpec::default())
            .cell(0, 0)
            .unwrap()
            .clone();
        assert_eq!(c.mean, None);
        assert_eq!((c.counted, c.rollouts, c.errored), (0, 1, 1));
    }

    #[test]
    fn missing_runtime_groups_under_not_recorded_and_filter_keeps_modal() {
        let rollouts = [
            r(
                "a",
                "t",
                Some("bash"),
                Some("qwen"),
                None,
                Outcome::Completed,
                Some(1.0),
            ),
            r(
                "b",
                "t",
                Some("bash"),
                Some("qwen"),
                Some("modal"),
                Outcome::Completed,
                Some(0.0),
            ),
            r(
                "c",
                "t",
                Some("bash"),
                Some("qwen"),
                Some("docker"),
                Outcome::Completed,
                Some(0.25),
            ),
        ];
        let spec = MatrixSpec {
            rows: Axis::Task,
            columns: vec![Axis::Runtime],
            filters: Vec::new(),
        };
        let m = matrix(&rollouts, &spec);
        assert_eq!(
            m.columns,
            [
                vec![NOT_RECORDED.to_owned()],
                vec!["modal".to_owned()],
                vec!["docker".to_owned()]
            ]
        );

        let filtered = matrix(
            &rollouts,
            &MatrixSpec {
                rows: Axis::Task,
                columns: vec![Axis::Runtime],
                filters: vec![(Axis::Runtime, "modal".into())],
            },
        );
        assert_eq!(filtered.columns, [vec!["modal".to_owned()]]);
        assert_eq!(filtered.cells.len(), 1);
        assert_eq!(filtered.cells[0].rollouts, 1);
        assert_eq!(filtered.cells[0].members, [1]);

        let absent = matrix(
            &rollouts,
            &MatrixSpec {
                rows: Axis::Task,
                columns: vec![Axis::Runtime],
                filters: vec![(Axis::Runtime, NOT_RECORDED.into())],
            },
        );
        assert_eq!(absent.columns, [vec![NOT_RECORDED.to_owned()]]);
        assert_eq!(absent.cells[0].members, [0]);
    }

    #[test]
    fn members_are_indices_into_the_input() {
        let rollouts = [
            r(
                "a",
                "t1",
                Some("bash"),
                Some("qwen"),
                None,
                Outcome::Completed,
                Some(1.0),
            ),
            r(
                "b",
                "t2",
                Some("bash"),
                Some("qwen"),
                None,
                Outcome::Completed,
                Some(0.0),
            ),
            r(
                "c",
                "t1",
                Some("bash"),
                Some("qwen"),
                None,
                Outcome::Completed,
                Some(0.5),
            ),
        ];
        let m = matrix(&rollouts, &MatrixSpec::default());
        assert_eq!(m.cell(0, 0).unwrap().members, [0, 2]);
        assert_eq!(m.cell(1, 0).unwrap().members, [1]);
    }
}
