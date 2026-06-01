//! Minimal Mermaid **flowchart** parser (`graph` / `flowchart`).
//!
//! Scope is deliberately just flowcharts — the most common Mermaid diagram in
//! markdown — and a pragmatic subset of their syntax: node shapes, directed and
//! undirected links, link labels, and chained edges. Anything we don't
//! understand (sequence diagrams, gantt, class, etc., or a malformed flowchart)
//! makes [`parse`] return `None`, and the caller degrades to showing the raw
//! source as a code block — the same "never break the document" contract charts
//! use.
//!
//! Parsing is pure and dependency-free; the rich reader rasterizes the parsed
//! [`Flowchart`] into the page (see `rich/mermaid_render.rs`), and the text
//! readers render an edge list.

/// Node outline shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `[text]`
    Rect,
    /// `(text)`
    Round,
    /// `([text])`
    Stadium,
    /// `{text}`
    Diamond,
    /// `((text))`
    Circle,
}

/// Layout flow direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Down,
    Up,
    Right,
    Left,
}

impl Dir {
    /// True for top-to-bottom / bottom-to-top (ranks stack vertically).
    /// (Used by the graphical renderer; the text reader ignores direction.)
    #[cfg_attr(not(feature = "rich"), allow(dead_code))]
    pub fn is_vertical(self) -> bool {
        matches!(self, Dir::Down | Dir::Up)
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    /// The node's mermaid id (graph identity / edge endpoint key). Retained for
    /// debugging and potential future use (e.g. anchors); rendering uses `label`.
    #[allow(dead_code)]
    pub id: String,
    pub label: String,
    pub shape: Shape,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    /// Whether the link ends in an arrowhead (`-->`) vs an open line (`---`).
    pub arrow: bool,
    /// Dotted (`-.->`) link — rendered dashed (graphical reader only).
    #[cfg_attr(not(feature = "rich"), allow(dead_code))]
    pub dashed: bool,
}

#[derive(Debug, Clone)]
pub struct Flowchart {
    /// Layout direction (graphical reader only; text reader emits an edge list).
    #[cfg_attr(not(feature = "rich"), allow(dead_code))]
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// Parse a Mermaid flowchart. Returns `None` if the block isn't a flowchart we
/// support (→ caller shows the source).
pub fn parse(src: &str) -> Option<Flowchart> {
    let mut lines = src.lines();

    // Header: first non-blank line must be `graph <DIR>` or `flowchart <DIR>`.
    let dir = loop {
        let line = lines.next()?;
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let kw = words.next()?.to_ascii_lowercase();
        if kw != "graph" && kw != "flowchart" {
            return None;
        }
        break match words.next().unwrap_or("TD").to_ascii_uppercase().as_str() {
            "TD" | "TB" | "V" => Dir::Down,
            "BT" => Dir::Up,
            "LR" => Dir::Right,
            "RL" => Dir::Left,
            _ => Dir::Down,
        };
    };

    let mut fc = Flowchart {
        dir,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for raw in lines {
        let line = strip_comment(raw);
        let line = line.trim().trim_end_matches(';').trim();
        if line.is_empty() || is_ignored_line(line) {
            continue;
        }
        if !parse_line(line, &mut fc, &mut index) {
            return None;
        }
    }

    if fc.nodes.is_empty() {
        return None;
    }
    Some(fc)
}

impl Flowchart {
    /// Longest-path layer assignment. Sources (no incoming edge) sit at rank 0;
    /// every other node sits one below its deepest predecessor.
    ///
    /// Cycles would make naive longest-path diverge (a back edge keeps pushing
    /// its target deeper each pass), so we first find back edges with a DFS and
    /// exclude them from ranking — the graph is ranked as the DAG you get by
    /// cutting cycles, which keeps the layout compact.
    ///
    /// Only the graphical renderer lays nodes out by rank; the text reader emits
    /// a flat edge list, so this is unused in `--no-default-features` builds.
    #[cfg_attr(not(feature = "rich"), allow(dead_code))]
    pub fn ranks(&self) -> Vec<usize> {
        let n = self.nodes.len();
        let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
        for (ei, e) in self.edges.iter().enumerate() {
            if e.from != e.to {
                adj[e.from].push((e.to, ei));
            }
        }

        // DFS colors: 0 = unvisited, 1 = on the current stack, 2 = done. An edge
        // to an on-stack node is a back edge (closes a cycle).
        let mut color = vec![0u8; n];
        let mut is_back = vec![false; self.edges.len()];
        for s in 0..n {
            if color[s] != 0 {
                continue;
            }
            color[s] = 1;
            let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
            while let Some(&(u, idx)) = stack.last() {
                if idx < adj[u].len() {
                    let (v, ei) = adj[u][idx];
                    stack.last_mut().unwrap().1 += 1;
                    match color[v] {
                        1 => is_back[ei] = true,
                        0 => {
                            color[v] = 1;
                            stack.push((v, 0));
                        }
                        _ => {}
                    }
                } else {
                    color[u] = 2;
                    stack.pop();
                }
            }
        }

        let mut rank = vec![0usize; n];
        for _ in 0..n {
            let mut changed = false;
            for (ei, e) in self.edges.iter().enumerate() {
                if is_back[ei] || e.from == e.to {
                    continue;
                }
                if rank[e.from] + 1 > rank[e.to] {
                    rank[e.to] = rank[e.from] + 1;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        rank
    }
}

/// Drop a `%% ...` trailing comment.
fn strip_comment(line: &str) -> &str {
    match line.find("%%") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Grouping/styling lines we parse-skip (they don't define graph topology).
fn is_ignored_line(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "subgraph" | "end" | "direction" | "classDef" | "class" | "style" | "click" | "linkStyle"
    )
}

/// Parse one topology line: `NODE (LINK NODE)*`, registering nodes and edges.
/// Returns `false` for malformed or unsupported topology so the caller can
/// fall back to rendering the Mermaid source instead of drawing a lie.
fn parse_line(
    line: &str,
    fc: &mut Flowchart,
    index: &mut std::collections::HashMap<String, usize>,
) -> bool {
    let mut pos = 0;
    let mut last: Option<usize> = None;
    let mut pending: Option<(bool, bool, Option<String>)> = None; // (arrow, dashed, label)
    let mut saw_node = false;

    while pos < line.len() {
        // Skip leading spaces.
        while pos < line.len() && matches!(line.as_bytes()[pos], b' ' | b'\t') {
            pos += 1;
        }
        if pos >= line.len() {
            break;
        }

        // Try a link first (so `A-->B` with no spaces also works).
        if let Some((arrow, dashed, label, consumed)) = parse_link(&line[pos..]) {
            if last.is_none() || pending.is_some() {
                return false;
            }
            pending = Some((arrow, dashed, label));
            pos += consumed;
            continue;
        }

        // Otherwise a node token.
        if let Some((idx, consumed)) = parse_node(&line[pos..], fc, index) {
            if let (Some(from), Some((arrow, dashed, label))) = (last, pending.take()) {
                fc.edges.push(Edge {
                    from,
                    to: idx,
                    label,
                    arrow,
                    dashed,
                });
            } else if last.is_some() {
                return false;
            }
            last = Some(idx);
            saw_node = true;
            pos += consumed;
        } else {
            return false;
        }
    }

    saw_node && pending.is_none()
}

/// Parse a node reference + optional shape at the start of `s`. Returns the node
/// index and bytes consumed.
fn parse_node(
    s: &str,
    fc: &mut Flowchart,
    index: &mut std::collections::HashMap<String, usize>,
) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    if b.is_empty() || !is_id_char(b[0]) {
        return None;
    }
    let mut i = 0;
    while i < b.len() && is_id_char(b[i]) {
        i += 1;
    }
    let id = &s[..i];

    // Optional shape wrapper immediately after the id.
    let (shape, label, after) = if let Some((open, close, shape)) = shape_delims(&s[i..]) {
        let body_start = i + open.len();
        if let Some(rel) = s[body_start..].find(close) {
            let raw = &s[body_start..body_start + rel];
            (
                shape,
                unquote(raw).to_string(),
                body_start + rel + close.len(),
            )
        } else {
            (Shape::Rect, id.to_string(), i)
        }
    } else {
        (Shape::Rect, id.to_string(), i)
    };

    let had_shape = after > i;
    let idx = *index.entry(id.to_string()).or_insert_with(|| {
        fc.nodes.push(Node {
            id: id.to_string(),
            label: label.clone(),
            shape,
        });
        fc.nodes.len() - 1
    });
    // A later reference that *does* carry a shape/label updates the node.
    if had_shape {
        fc.nodes[idx].label = label;
        fc.nodes[idx].shape = shape;
    }
    Some((idx, after))
}

/// Recognize a shape opener at the start of `s`, returning (open, close, shape).
/// Longer openers are tested first so `([` / `((` win over `(`.
fn shape_delims(s: &str) -> Option<(&'static str, &'static str, Shape)> {
    const TABLE: &[(&str, &str, Shape)] = &[
        ("([", "])", Shape::Stadium),
        ("((", "))", Shape::Circle),
        ("[(", ")]", Shape::Stadium), // cylinder/database — render as stadium
        ("[[", "]]", Shape::Rect),
        ("[", "]", Shape::Rect),
        ("(", ")", Shape::Round),
        ("{", "}", Shape::Diamond),
    ];
    TABLE
        .iter()
        .find(|(open, _, _)| s.starts_with(*open))
        .map(|&(o, c, sh)| (o, c, sh))
}

/// Edge operators, longest first (so `-->` beats `--`, `-.->` beats `-.-`).
const ARROWS: &[&str] = &[
    "<-->", "-.->", "x-->", "o-->", "==>", "===", "-.-", "--x", "--o", "-->", "---",
];

/// Parse a link at the start of `s`. Handles `A-->B`, `A -->|label| B`, and the
/// middle-text forms `A -- label --> B` / `A == label ==> B` / `A -. label .-> B`.
/// Returns `(arrow, dashed, label, bytes_consumed)`.
fn parse_link(s: &str) -> Option<(bool, bool, Option<String>, usize)> {
    let b = s.as_bytes();
    if b.is_empty() || !matches!(b[0], b'-' | b'=' | b'<' | b'x' | b'o') {
        return None;
    }

    // Direct operator (optionally followed by a `|label|`).
    if let Some(op) = ARROWS.iter().find(|op| s.starts_with(**op)) {
        let mut consumed = op.len();
        let mut label = None;
        if s[consumed..].starts_with('|')
            && let Some(rel) = s[consumed + 1..].find('|')
        {
            label = Some(s[consumed + 1..consumed + 1 + rel].trim().to_string());
            consumed += 1 + rel + 1;
        }
        return Some((
            op.ends_with('>') || op.contains('x') || op.contains('o'),
            op.contains('.'),
            label,
            consumed,
        ));
    }

    // Middle-text form: a cap, a space, label text, then a closing operator.
    for (cap, dashed) in [("-.", true), ("==", false), ("--", false)] {
        if s.starts_with(cap) && s[cap.len()..].starts_with(' ') {
            let rest = &s[cap.len()..];
            for (j, _) in rest.char_indices() {
                if let Some(op) = ARROWS.iter().find(|op| rest[j..].starts_with(**op)) {
                    let label = rest[..j].trim();
                    let label = (!label.is_empty()).then(|| label.to_string());
                    let arrow = op.ends_with('>') || op.contains('x') || op.contains('o');
                    return Some((
                        arrow,
                        dashed || op.contains('.'),
                        label,
                        cap.len() + j + op.len(),
                    ));
                }
            }
        }
    }
    None
}

fn unquote(s: &str) -> &str {
    let t = s.trim();
    t.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(t)
}

fn is_id_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_flowchart_returns_none() {
        assert!(parse("sequenceDiagram\n A->>B: hi").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn parses_nodes_edges_shapes() {
        let fc = parse("graph TD\n  A[Start] --> B{OK?}\n  B -->|yes| C(Done)\n  B --> D[Stop]")
            .expect("flowchart");
        assert_eq!(fc.dir, Dir::Down);
        assert_eq!(fc.nodes.len(), 4);
        assert_eq!(fc.nodes[0].label, "Start");
        assert_eq!(fc.nodes[1].shape, Shape::Diamond);
        assert_eq!(fc.nodes[2].shape, Shape::Round);
        assert_eq!(fc.edges.len(), 3);
        // The labeled edge B -->|yes| C
        let labeled = fc
            .edges
            .iter()
            .find(|e| e.label.as_deref() == Some("yes"))
            .unwrap();
        assert_eq!(fc.nodes[labeled.to].label, "Done");
    }

    #[test]
    fn chained_edges_and_lr() {
        let fc = parse("flowchart LR\n A --> B --> C").unwrap();
        assert_eq!(fc.dir, Dir::Right);
        assert_eq!(fc.nodes.len(), 3);
        assert_eq!(fc.edges.len(), 2);
    }

    #[test]
    fn middle_text_link_label() {
        let fc = parse("graph TD\n A -- holds --> B").unwrap();
        assert_eq!(fc.edges.len(), 1);
        assert_eq!(fc.edges[0].label.as_deref(), Some("holds"));
    }

    #[test]
    fn malformed_topology_falls_back_to_source() {
        assert!(parse("graph TD\n A -->").is_none());
        assert!(parse("graph TD\n A ??? B").is_none());
        assert!(parse("graph TD\n A B").is_none());
    }

    #[test]
    fn accepts_trailing_semicolon() {
        let fc = parse("graph TD\n A --> B;").unwrap();
        assert_eq!(fc.edges.len(), 1);
    }

    #[test]
    fn ranks_are_longest_path() {
        let fc = parse("graph TD\n A --> B\n B --> C\n A --> C").unwrap();
        let r = fc.ranks();
        // C must sit below B (longest path A->B->C = rank 2), not at rank 1.
        assert_eq!(r[0], 0);
        assert_eq!(r[1], 1);
        assert_eq!(r[2], 2);
    }

    #[test]
    fn cycle_terminates() {
        let fc = parse("graph LR\n A --> B\n B --> A").unwrap();
        let _ = fc.ranks(); // must not loop forever
        assert_eq!(fc.edges.len(), 2);
    }
}
