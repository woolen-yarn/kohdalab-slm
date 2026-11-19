//! One-axis sweeps. Values are computed from the start by index, never by
//! repeated addition, and a descending range uses the same positive step.

use crate::pattern::PatternSpec;
use crate::units::{PeriodUnit, format_number};
use serde::{Deserialize, Serialize};

pub const MAX_SWEEP_POINTS: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum SweepTarget {
    /// Raw contrast level of a Uniform pattern.
    UniformCode,
    /// Period/wavevector of one component, in that component's entry unit.
    Period { component: usize },
    /// Initial phase of one component, in that component's entry unit.
    InitialPhase { component: usize },
    /// Common rotation angle, in the pattern's entry unit.
    Rotation,
}

impl SweepTarget {
    pub fn label(&self, base: &PatternSpec) -> String {
        let unit = self.unit(base).unwrap_or_default();
        let multi = matches!(base, PatternSpec::MultiWave { .. });
        match self {
            SweepTarget::UniformCode => "Contrast level [code]".into(),
            SweepTarget::Period { component } => {
                let name = base
                    .components()
                    .get(*component)
                    .map(|c| c.period.unit)
                    .unwrap_or(PeriodUnit::SlmPx)
                    .label();
                if multi {
                    format!("Component {} {name}", component + 1)
                } else {
                    name.into()
                }
            }
            SweepTarget::InitialPhase { component } if multi => {
                format!("Component {} initial phase [{unit}]", component + 1)
            }
            SweepTarget::InitialPhase { .. } => format!("Initial phase [{unit}]"),
            SweepTarget::Rotation => format!("Rotation [{unit}]"),
        }
    }
    pub fn unit(&self, base: &PatternSpec) -> Option<String> {
        Some(match self {
            SweepTarget::UniformCode => "code".into(),
            SweepTarget::Period { component } => base
                .components()
                .get(*component)?
                .period
                .unit
                .suffix()
                .into(),
            SweepTarget::InitialPhase { component } => base
                .components()
                .get(*component)?
                .initial_phase
                .unit
                .suffix()
                .into(),
            SweepTarget::Rotation => base.geometry()?.rotation.unit.suffix().into(),
        })
    }
    /// Value of the target in `spec`, in the target's entry unit.
    pub fn value(&self, spec: &PatternSpec) -> Option<f64> {
        match (self, spec) {
            (SweepTarget::UniformCode, PatternSpec::Uniform { code }) => Some(f64::from(*code)),
            (SweepTarget::Period { component }, _) => {
                Some(spec.components().get(*component)?.period.value)
            }
            (SweepTarget::InitialPhase { component }, _) => {
                Some(spec.components().get(*component)?.initial_phase.value)
            }
            (SweepTarget::Rotation, _) => Some(spec.geometry()?.rotation.value),
            _ => None,
        }
    }
    fn apply(&self, spec: &mut PatternSpec, value: f64) -> Result<(), String> {
        match (self, &mut *spec) {
            (SweepTarget::UniformCode, PatternSpec::Uniform { code }) => {
                if value.fract() != 0. || !(0. ..=f64::from(u16::MAX)).contains(&value) {
                    return Err(format!(
                        "Contrast level {value} is not a valid integer code"
                    ));
                }
                *code = value as u16;
            }
            (SweepTarget::UniformCode, _) => {
                return Err("Contrast level sweeps need a Uniform base pattern".into());
            }
            (SweepTarget::Period { component }, _) => {
                spec.components_mut()
                    .get_mut(*component)
                    .ok_or("Swept component does not exist")?
                    .period
                    .value = value;
            }
            (SweepTarget::InitialPhase { component }, _) => {
                spec.components_mut()
                    .get_mut(*component)
                    .ok_or("Swept component does not exist")?
                    .initial_phase
                    .value = value;
            }
            (SweepTarget::Rotation, _) => {
                spec.geometry_mut()
                    .ok_or("Uniform patterns have no rotation")?
                    .rotation
                    .value = value;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SweepSpec {
    pub base: PatternSpec,
    pub target: SweepTarget,
    pub start: f64,
    pub end: f64,
    /// Positive step magnitude; the direction follows `start → end`.
    pub step: f64,
    /// Append `end` when it does not fall on the step grid.
    pub include_end: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SweepPreview {
    pub values: Vec<f64>,
    pub first: f64,
    pub last: f64,
    pub count: usize,
    /// `end` was not on the grid (and was appended only if `include_end`).
    pub end_off_grid: bool,
}

impl SweepPreview {
    pub fn describe(&self, unit: &str) -> String {
        format!(
            "{} → {} {unit}, {} item{}{}",
            format_number(self.first),
            format_number(self.last),
            self.count,
            if self.count == 1 { "" } else { "s" },
            if self.end_off_grid {
                " (end value is not on the step grid)"
            } else {
                ""
            }
        )
    }
}

/// Number of decimal digits in the shortest representation, if small.
fn decimals(value: f64) -> Option<u32> {
    let text = format!("{value}");
    if text.contains('e') {
        return None;
    }
    let d = text.split_once('.').map_or(0, |(_, f)| f.len() as u32);
    (d <= 9).then_some(d)
}

/// Index grid shared by the full value list and the light summary, so both
/// give identical numbers.
struct Grid {
    start: f64,
    end: f64,
    step: f64,
    direction: f64,
    intervals: u64,
    scale: Option<f64>,
}

impl Grid {
    fn new(start: f64, end: f64, step: f64) -> Result<Self, String> {
        if !(start.is_finite() && end.is_finite()) {
            return Err("Start and end must be finite".into());
        }
        if !(step.is_finite() && step > 0.) {
            return Err("Step must be a positive number".into());
        }
        let direction = if end >= start { 1. } else { -1. };
        let ratio = (end - start).abs() / step;
        let tolerance = 1e-9 * ratio.max(1.);
        let intervals = (ratio + tolerance).floor().max(0.);
        if intervals + 1. > MAX_SWEEP_POINTS as f64 {
            return Err(format!(
                "Sweep would create {} items; the limit is {MAX_SWEEP_POINTS}",
                intervals + 1.
            ));
        }
        // Exact decimal grid when the entries are short decimals (0.1 → 0.3,
        // not 0.30000000000000004); otherwise the plain indexed formula.
        let scale = match (decimals(start), decimals(step)) {
            (Some(a), Some(b)) if a.max(b) <= 9 => Some(10f64.powi(a.max(b) as i32)),
            _ => None,
        };
        Ok(Self {
            start,
            end,
            step,
            direction,
            intervals: intervals as u64,
            scale,
        })
    }
    fn at(&self, i: u64) -> f64 {
        let i = i as f64;
        let (start, step) = (self.start, self.step);
        match self.scale {
            Some(scale) if (start * scale).abs() < 9e15 && (step * scale * i).abs() < 9e15 => {
                ((start * scale).round() + self.direction * i * (step * scale).round()) / scale
            }
            _ => start + self.direction * i * step,
        }
    }
    fn end_on_grid(&self) -> bool {
        (self.at(self.intervals) - self.end).abs() <= 1e-9 * self.step.max(self.end.abs())
    }
}

pub fn sweep_values(
    start: f64,
    end: f64,
    step: f64,
    include_end: bool,
) -> Result<SweepPreview, String> {
    let grid = Grid::new(start, end, step)?;
    let mut values: Vec<f64> = (0..=grid.intervals).map(|i| grid.at(i)).collect();
    let end_on_grid = grid.end_on_grid();
    if end_on_grid {
        *values.last_mut().expect("non-empty") = end;
    } else if include_end {
        values.push(end);
    }
    Ok(SweepPreview {
        first: values[0],
        last: *values.last().expect("non-empty"),
        count: values.len(),
        end_off_grid: !end_on_grid,
        values,
    })
}

/// Same first / last / count as [`sweep_values`] without building the list
/// (`values` is empty). Cheap enough to call every frame.
pub fn sweep_summary(
    start: f64,
    end: f64,
    step: f64,
    include_end: bool,
) -> Result<SweepPreview, String> {
    let grid = Grid::new(start, end, step)?;
    let end_on_grid = grid.end_on_grid();
    let appended = !end_on_grid && include_end;
    let first = if grid.intervals == 0 && end_on_grid {
        end
    } else {
        grid.at(0)
    };
    let last = if end_on_grid || appended {
        end
    } else {
        grid.at(grid.intervals)
    };
    Ok(SweepPreview {
        first,
        last,
        count: grid.intervals as usize + 1 + usize::from(appended),
        end_off_grid: !end_on_grid,
        values: Vec::new(),
    })
}

impl SweepSpec {
    fn check(&self) -> Result<(), String> {
        if self.target.value(&self.base).is_none() {
            return Err("The sweep target does not exist in the base pattern".into());
        }
        if matches!(self.target, SweepTarget::UniformCode)
            && (self.start.fract() != 0. || self.end.fract() != 0. || self.step.fract() != 0.)
        {
            return Err("Contrast level sweeps use integer start, end and step".into());
        }
        Ok(())
    }

    /// First / last / count without expanding the sweep.
    pub fn summary(&self) -> Result<SweepPreview, String> {
        self.check()?;
        sweep_summary(self.start, self.end, self.step, self.include_end)
    }

    /// The first sweep pattern, built with the same `target.apply` as
    /// [`SweepSpec::expand`] but without generating every value.
    pub fn first_pattern(&self) -> Result<(f64, PatternSpec), String> {
        let first = self.summary()?.first;
        let mut spec = self.base.clone();
        self.target.apply(&mut spec, first)?;
        Ok((first, spec))
    }

    pub fn preview(&self) -> Result<SweepPreview, String> {
        self.check()?;
        sweep_values(self.start, self.end, self.step, self.include_end)
    }
    /// One pattern per value; other fields keep the base values.
    pub fn expand(&self) -> Result<Vec<(f64, PatternSpec)>, String> {
        let preview = self.preview()?;
        preview
            .values
            .iter()
            .map(|&value| {
                let mut spec = self.base.clone();
                self.target.apply(&mut spec, value)?;
                Ok((value, spec))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_examples() {
        let c = sweep_values(0., 1023., 10., false).unwrap();
        assert_eq!((c.count, c.first, c.last), (103, 0., 1020.));
        assert!(c.end_off_grid);
        let c = sweep_values(0., 1023., 10., true).unwrap();
        assert_eq!((c.count, c.last), (104, 1023.));
        let q = sweep_values(0., 1.0, 0.1, false).unwrap();
        assert_eq!(q.count, 11);
        assert_eq!(q.values[3], 0.3);
        assert_eq!(q.last, 1.0);
        assert!(!q.end_off_grid);
        let r = sweep_values(0., 90., 10., false).unwrap();
        assert_eq!(r.count, 10);
        assert_eq!(
            r.values,
            (0..10).map(|i| f64::from(i) * 10.).collect::<Vec<_>>()
        );
    }

    #[test]
    fn descending_with_positive_step() {
        let d = sweep_values(1.0, 0.0, 0.25, false).unwrap();
        assert_eq!(d.values, vec![1.0, 0.75, 0.5, 0.25, 0.0]);
        let d = sweep_values(10., 0., 3., true).unwrap();
        assert_eq!(d.values, vec![10., 7., 4., 1., 0.]);
        assert!(sweep_values(0., 1., 0., false).is_err());
        assert!(sweep_values(0., 1., -0.1, false).is_err());
        let single = sweep_values(5., 5., 1., true).unwrap();
        assert_eq!(single.values, vec![5.]);
    }

    #[test]
    fn summary_and_first_pattern_match_full_expansion() {
        let cases = [
            (0., 1023., 10., false),
            (0., 1023., 10., true),
            (0., 1., 0.1, false),
            (0., 90., 10., false),
            (90., 0., 30., true),
            (10., 0., 3., true),
            (5., 5., 1., true),
            (0.25, 0.3, 0.1, true),
        ];
        for (start, end, step, include_end) in cases {
            let full = sweep_values(start, end, step, include_end).unwrap();
            let light = sweep_summary(start, end, step, include_end).unwrap();
            assert_eq!(
                (light.first, light.last, light.count, light.end_off_grid),
                (full.first, full.last, full.count, full.end_off_grid),
                "{start} {end} {step} {include_end}"
            );
        }
        let spec = SweepSpec {
            base: PatternSpec::Uniform { code: 7 },
            target: SweepTarget::UniformCode,
            start: 40.,
            end: 0.,
            step: 10.,
            include_end: false,
        };
        assert_eq!(spec.first_pattern().unwrap(), spec.expand().unwrap()[0]);
        assert!(SweepSpec { step: 0.5, ..spec }.first_pattern().is_err());
    }

    #[test]
    fn expand_changes_only_target() {
        let base = PatternSpec::Uniform { code: 7 };
        let spec = SweepSpec {
            base,
            target: SweepTarget::UniformCode,
            start: 0.,
            end: 1023.,
            step: 10.,
            include_end: true,
        };
        let items = spec.expand().unwrap();
        assert_eq!(items.len(), 104);
        assert_eq!(items[103].1, PatternSpec::Uniform { code: 1023 });
        let bad = SweepSpec { step: 0.5, ..spec };
        assert!(bad.preview().is_err());
    }
}
