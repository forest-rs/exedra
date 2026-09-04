// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic v1 golden snapshot text format.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

const HEADER: &str = "exedra-ops-golden-v1";

/// One scenario snapshot containing ordered operator steps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoldenSnapshot {
    /// Stable scenario label.
    pub scenario: String,
    /// Ordered steps retained in golden output.
    pub steps: Vec<GoldenStep>,
}

/// One operator step in a golden scenario.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoldenStep {
    /// Step label.
    pub label: String,
    /// Operator name.
    pub op: String,
    /// Faces-processed counter.
    pub faces_processed: u64,
    /// Corners-written counter.
    pub corners_written: u64,
    /// Selections-canonicalized counter.
    pub selections_canonicalized: u64,
    /// Ordered diagnostics.
    pub diagnostics: Vec<String>,
}

/// Parse error for [`parse_snapshot`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    /// 1-based line number.
    pub line: usize,
    /// Error reason.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl core::error::Error for ParseError {}

/// Renders a deterministic text snapshot.
#[must_use]
pub fn render_snapshot(snapshot: &GoldenSnapshot) -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    out.push('\n');
    out.push_str("scenario ");
    out.push_str(&snapshot.scenario);
    out.push('\n');
    for step in &snapshot.steps {
        out.push_str("step ");
        out.push_str(&step.label);
        out.push('\n');
        out.push_str("op ");
        out.push_str(&step.op);
        out.push('\n');
        out.push_str("faces_processed ");
        out.push_str(&step.faces_processed.to_string());
        out.push('\n');
        out.push_str("corners_written ");
        out.push_str(&step.corners_written.to_string());
        out.push('\n');
        out.push_str("selections_canonicalized ");
        out.push_str(&step.selections_canonicalized.to_string());
        out.push('\n');
        for diag in &step.diagnostics {
            out.push_str("diag ");
            out.push_str(diag);
            out.push('\n');
        }
        out.push_str("end\n");
    }
    out
}

/// Parses a text snapshot encoded by [`render_snapshot`].
pub fn parse_snapshot(input: &str) -> Result<GoldenSnapshot, ParseError> {
    let mut lines = input.lines().enumerate();
    let Some((header_line, header)) = lines.next() else {
        return Err(ParseError {
            line: 1,
            message: "missing header".into(),
        });
    };
    if header != HEADER {
        return Err(ParseError {
            line: header_line + 1,
            message: "invalid header".into(),
        });
    }

    let Some((scenario_line, scenario_record)) = lines.next() else {
        return Err(ParseError {
            line: 2,
            message: "missing scenario record".into(),
        });
    };
    let Some(scenario) = scenario_record.strip_prefix("scenario ") else {
        return Err(ParseError {
            line: scenario_line + 1,
            message: "expected `scenario <name>`".into(),
        });
    };

    let mut steps = Vec::new();
    while let Some((line_no, line)) = lines.next() {
        if line.is_empty() {
            continue;
        }
        let Some(label) = line.strip_prefix("step ") else {
            return Err(ParseError {
                line: line_no + 1,
                message: "expected `step <label>`".into(),
            });
        };

        let (_, op) = parse_prefixed_record(&mut lines, "op ", "operator name")?;
        let faces_processed = parse_u64_record(&mut lines, "faces_processed ", "faces_processed")?;
        let corners_written = parse_u64_record(&mut lines, "corners_written ", "corners_written")?;
        let selections_canonicalized = parse_u64_record(
            &mut lines,
            "selections_canonicalized ",
            "selections_canonicalized",
        )?;

        let mut diagnostics = Vec::new();
        loop {
            let Some((diag_line_no, diag_line)) = lines.next() else {
                return Err(ParseError {
                    line: line_no + 1,
                    message: "unterminated step (missing `end`)".into(),
                });
            };
            if diag_line == "end" {
                break;
            }
            let Some(diag) = diag_line.strip_prefix("diag ") else {
                return Err(ParseError {
                    line: diag_line_no + 1,
                    message: "expected `diag <message>` or `end`".into(),
                });
            };
            diagnostics.push(diag.to_string());
        }

        steps.push(GoldenStep {
            label: label.to_string(),
            op,
            faces_processed,
            corners_written,
            selections_canonicalized,
            diagnostics,
        });
    }

    Ok(GoldenSnapshot {
        scenario: scenario.to_string(),
        steps,
    })
}

fn parse_prefixed_record(
    lines: &mut core::iter::Enumerate<core::str::Lines<'_>>,
    prefix: &str,
    field: &str,
) -> Result<(usize, String), ParseError> {
    let Some((line_no, line)) = lines.next() else {
        return Err(ParseError {
            line: 1,
            message: format!("missing {field}"),
        });
    };
    let Some(value) = line.strip_prefix(prefix) else {
        return Err(ParseError {
            line: line_no + 1,
            message: format!("expected `{prefix}...`"),
        });
    };
    Ok((line_no + 1, value.to_string()))
}

fn parse_u64_record(
    lines: &mut core::iter::Enumerate<core::str::Lines<'_>>,
    prefix: &str,
    field: &str,
) -> Result<u64, ParseError> {
    let (line, value) = parse_prefixed_record(lines, prefix, field)?;
    value.parse::<u64>().map_err(|_| ParseError {
        line,
        message: format!("invalid integer for {field}"),
    })
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec;

    use super::{GoldenSnapshot, GoldenStep, parse_snapshot, render_snapshot};

    #[test]
    fn snapshot_roundtrips() {
        let snapshot = GoldenSnapshot {
            scenario: "basilica".to_string(),
            steps: vec![
                GoldenStep {
                    label: "step_1".to_string(),
                    op: "inspect.validate.mesh".to_string(),
                    faces_processed: 4,
                    corners_written: 0,
                    selections_canonicalized: 0,
                    diagnostics: vec!["ok".to_string()],
                },
                GoldenStep {
                    label: "step_2".to_string(),
                    op: "tag.face.region".to_string(),
                    faces_processed: 1,
                    corners_written: 0,
                    selections_canonicalized: 1,
                    diagnostics: vec![],
                },
            ],
        };
        let encoded = render_snapshot(&snapshot);
        let decoded = parse_snapshot(&encoded).expect("snapshot should parse");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn parser_rejects_bad_header() {
        let encoded = "bad-header\nscenario s\n";
        let error = parse_snapshot(encoded).expect_err("must reject bad header");
        assert_eq!(error.line, 1);
    }
}
