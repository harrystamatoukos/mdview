//! Chart spec parsing for ` ```chart ` fenced code blocks.
//!
//! The block body is a small YAML document (see the mdview presentation
//! convention). This module owns only the *data model* and its parsing — no
//! rendering — so it stays available in every build (the rich reader rasterizes
//! with `plotters`, the text reader draws bars, both read this same `Chart`).
//!
//! Parsing is deliberately lenient: anything that doesn't resolve to a valid
//! chart returns `None`, and the caller falls back to rendering the block as an
//! ordinary code block. That preserves the convention's fallback contract — a
//! malformed chart degrades to its legible YAML rather than an error.

use indexmap::IndexMap;
use serde::Deserialize;

/// The four chart types the convention defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    Bar,
    Line,
    Scatter,
    Pie,
}

/// One named data series (a single bar/line dataset).
#[derive(Debug, Clone)]
pub struct Series {
    pub name: String,
    pub y: Vec<f64>,
}

/// A parsed, validated chart ready to render.
#[derive(Debug, Clone)]
pub struct Chart {
    pub kind: ChartKind,
    pub title: Option<String>,
    pub xlabel: Option<String>,
    pub ylabel: Option<String>,
    /// Category labels (stringified) for bar/line x-axis.
    pub x: Vec<String>,
    /// Series for bar/line. A bare `y:` becomes a single unnamed series.
    pub series: Vec<Series>,
    /// Scatter points.
    pub points: Vec<(f64, f64)>,
    /// Pie slices, in source order.
    pub data: Vec<(String, f64)>,
}

impl Chart {
    /// Parse a chart YAML body. Returns `None` if it isn't a valid chart, so the
    /// caller can fall back to a plain code block.
    pub fn parse(yaml: &str) -> Option<Chart> {
        let raw: Raw = serde_yaml_ng::from_str(yaml).ok()?;

        let x = raw
            .x
            .unwrap_or_default()
            .iter()
            .map(Scalar::label)
            .collect();

        let series = if let Some(series) = raw.series {
            series
                .into_iter()
                .map(|s| Series {
                    name: s.name,
                    y: s.y,
                })
                .collect()
        } else if let Some(y) = raw.y {
            vec![Series {
                name: String::new(),
                y,
            }]
        } else {
            Vec::new()
        };

        let points = raw
            .points
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p[0], p[1]))
            .collect();

        let data = raw.data.unwrap_or_default().into_iter().collect();

        let chart = Chart {
            kind: raw.kind,
            title: raw.title,
            xlabel: raw.xlabel,
            ylabel: raw.ylabel,
            x,
            series,
            points,
            data,
        };

        chart.is_valid().then_some(chart)
    }

    /// Whether the chart carries the data its kind needs to render.
    fn is_valid(&self) -> bool {
        match self.kind {
            ChartKind::Bar | ChartKind::Line => {
                !self.series.is_empty() && self.series.iter().all(|s| !s.y.is_empty())
            }
            ChartKind::Scatter => !self.points.is_empty(),
            ChartKind::Pie => !self.data.is_empty(),
        }
    }

    /// Largest absolute y across all series (bar/line axis scaling). At least 1.
    pub fn y_max(&self) -> f64 {
        self.series
            .iter()
            .flat_map(|s| s.y.iter())
            .fold(0.0_f64, |m, &v| m.max(v))
            .max(f64::MIN_POSITIVE)
    }
}

/// Format a number as a compact label: integers drop the decimal.
pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        // Trim to at most 2 decimals without trailing zeros.
        let s = format!("{n:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// ── Raw deserialization shapes (forgiving) ─────────────────────────────────

#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "type")]
    kind: ChartKind,
    title: Option<String>,
    xlabel: Option<String>,
    ylabel: Option<String>,
    x: Option<Vec<Scalar>>,
    y: Option<Vec<f64>>,
    series: Option<Vec<RawSeries>>,
    points: Option<Vec<[f64; 2]>>,
    data: Option<IndexMap<String, f64>>,
}

#[derive(Deserialize)]
struct RawSeries {
    name: String,
    y: Vec<f64>,
}

/// An x-axis entry that may be written as a number or a string in YAML.
#[derive(Deserialize)]
#[serde(untagged)]
enum Scalar {
    Num(f64),
    Str(String),
}

impl Scalar {
    fn label(&self) -> String {
        match self {
            Scalar::Num(n) => fmt_num(*n),
            Scalar::Str(s) => s.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_series_bar() {
        let c = Chart::parse("type: bar\nx: [Mon, Tue]\ny: [1, 2]").unwrap();
        assert_eq!(c.kind, ChartKind::Bar);
        assert_eq!(c.x, vec!["Mon", "Tue"]);
        assert_eq!(c.series.len(), 1);
        assert_eq!(c.series[0].y, vec![1.0, 2.0]);
    }

    #[test]
    fn parses_numeric_x_as_labels() {
        let c = Chart::parse("type: line\nx: [1, 2, 3]\ny: [4, 5, 6]").unwrap();
        assert_eq!(c.x, vec!["1", "2", "3"]);
    }

    #[test]
    fn parses_multi_series() {
        let y =
            "type: line\nx: [1,2]\nseries:\n  - name: A\n    y: [1,2]\n  - name: B\n    y: [3,4]";
        let c = Chart::parse(y).unwrap();
        assert_eq!(c.series.len(), 2);
        assert_eq!(c.series[1].name, "B");
    }

    #[test]
    fn parses_pie_in_order() {
        let c = Chart::parse("type: pie\ndata:\n  A: 40\n  B: 25\n  C: 35").unwrap();
        assert_eq!(
            c.data,
            vec![("A".into(), 40.0), ("B".into(), 25.0), ("C".into(), 35.0)]
        );
    }

    #[test]
    fn parses_scatter() {
        let c = Chart::parse("type: scatter\npoints: [[1, 2], [3, 4]]").unwrap();
        assert_eq!(c.points, vec![(1.0, 2.0), (3.0, 4.0)]);
    }

    #[test]
    fn rejects_missing_data() {
        assert!(Chart::parse("type: bar\ntitle: empty").is_none());
        assert!(Chart::parse("type: pie").is_none());
    }

    #[test]
    fn rejects_non_chart_yaml() {
        assert!(Chart::parse("just: some\nrandom: yaml").is_none());
        assert!(Chart::parse("not yaml at all: [unclosed").is_none());
    }
}
