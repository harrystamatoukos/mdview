//! Pure file-tree model for the directory sidebar.
//!
//! Builds a recursive tree of the `.md`/`.markdown` files under a root
//! directory, pruning any directory that has no markdown descendant. The tree
//! is flattened into a list of *visible rows* according to which directories
//! are expanded; the reader's sidebar renders that list and drives selection
//! through it.
//!
//! No graphics or terminal dependencies — this is plain logic so it can be unit
//! tested and reused by other front-ends.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// A node in the file tree. Directories carry their (already-pruned, sorted)
/// children; files are leaves. Expansion state lives in [`FileTree::expanded`]
/// rather than here, so toggling never needs a mutable tree walk.
#[derive(Debug, Clone)]
struct Node {
    name: String,
    path: PathBuf,
    is_dir: bool,
    children: Vec<Node>,
}

/// One flattened, on-screen row: a file or directory at a given indent depth.
#[derive(Debug, Clone)]
pub struct VisibleRow {
    pub depth: u16,
    pub is_dir: bool,
    /// For directories: whether currently expanded (drives the chevron glyph).
    pub expanded: bool,
    pub name: String,
    pub path: PathBuf,
    /// A short, dimmed locator shown after the name. `None` for tree rows; for
    /// search results it holds the parent dir (relative to root) plus a match
    /// count, e.g. `guide/advanced · 3`. Pre-truncated at construction.
    pub detail: Option<String>,
}

/// Outcome of running a search: how many files matched and whether the scan was
/// capped before reaching the end of the tree.
#[derive(Debug, Clone, Copy)]
pub struct SearchOutcome {
    /// Number of files matched. Read in tests; the UI uses the live row count.
    #[allow(dead_code)]
    pub matched: usize,
    pub truncated: bool,
}

/// Stop scanning after this many files (keeps a synchronous search on the UI
/// thread bounded even in a large, un-pruned tree).
const MAX_SEARCH_FILES: usize = 2000;
/// Skip files larger than this when grepping contents (filename can still match).
const MAX_SEARCH_FILE_BYTES: u64 = 1_000_000;
/// Cap on the dimmed locator length so a deep path can't blow up the row width.
const MAX_DETAIL_LEN: usize = 40;

/// The browsable tree plus selection and scroll state.
pub struct FileTree {
    root: Node,
    expanded: HashSet<PathBuf>,
    selected: usize,
    visible: Vec<VisibleRow>,
    /// When `Some`, the sidebar shows these ranked search results instead of the
    /// tree. `selected`/`scroll` index into whichever list `visible()` returns.
    search: Option<Vec<VisibleRow>>,
    /// Index of the first visible row currently shown (for list scrolling).
    scroll: usize,
}

/// Whether a path looks like a markdown file by extension.
fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md") | Some("markdown")
    )
}

/// Recursively scan `dir`, returning a directory node *only if* it (transitively)
/// contains at least one markdown file. Children are sorted directories-first,
/// then files, each alphabetically (case-insensitive). Unreadable entries are
/// skipped rather than failing the whole scan.
fn scan(dir: &Path) -> Option<Node> {
    let mut children: Vec<Node> = Vec::new();
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        // Skip hidden entries (dotfiles / dot-directories).
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        if file_type.is_dir() {
            if let Some(node) = scan(&path) {
                children.push(node);
            }
        } else if file_type.is_file() && is_markdown(&path) {
            children.push(Node {
                name: entry.file_name().to_string_lossy().into_owned(),
                path,
                is_dir: false,
                children: Vec::new(),
            });
        }
    }

    if children.is_empty() {
        return None;
    }
    sort_children(&mut children);
    Some(Node {
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned()),
        path: dir.to_path_buf(),
        is_dir: true,
        children,
    })
}

/// Directories first, then files; alphabetical (case-insensitive) within each.
fn sort_children(children: &mut [Node]) {
    children.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
}

impl FileTree {
    /// Build the tree rooted at `root_dir`. If `focus` is given and lies within
    /// the tree, its ancestor directories are expanded and it is selected;
    /// otherwise the top level is shown and the first file is selected.
    pub fn build(root_dir: &Path, focus: Option<&Path>) -> Result<Self> {
        // An empty root (no markdown anywhere) is still a valid, empty tree.
        let root = scan(root_dir).unwrap_or_else(|| Node {
            name: root_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root_dir.to_string_lossy().into_owned()),
            path: root_dir.to_path_buf(),
            is_dir: true,
            children: Vec::new(),
        });

        let mut tree = FileTree {
            root,
            expanded: HashSet::new(),
            selected: 0,
            visible: Vec::new(),
            search: None,
            scroll: 0,
        };

        // Expand the ancestor chain of the focus file so it's revealed, then
        // select it.
        if let Some(focus) = focus {
            tree.expand_to(focus);
        }
        tree.rebuild_visible();
        if let Some(focus) = focus
            && let Some(idx) = tree.visible.iter().position(|r| r.path == focus)
        {
            tree.selected = idx;
        }
        Ok(tree)
    }

    /// Expand the ancestor directory chain of `path` so a `rebuild_visible` will
    /// reveal it. `path` and the tree share a path space (both derived from the
    /// same CLI argument), so no canonicalization is needed.
    fn expand_to(&mut self, path: &Path) {
        let mut cur = path.parent();
        while let Some(dir) = cur {
            self.expanded.insert(dir.to_path_buf());
            if dir == self.root.path {
                break;
            }
            cur = dir.parent();
        }
    }

    /// Whether the tree has no rows at all (browse mode).
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Whether a search is currently active (results are shown instead of the tree).
    pub fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    /// The visible rows currently on screen — search results when a search is
    /// active, else the tree's flattened rows. The caller windows it with
    /// [`scroll`](Self::scroll) and [`ensure_visible`](Self::ensure_visible).
    pub fn visible(&self) -> &[VisibleRow] {
        self.search.as_deref().unwrap_or(&self.visible)
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Path of the currently selected row, if any.
    #[allow(dead_code)] // exercised by tests; part of the tree's public surface
    pub fn selected_path(&self) -> Option<&Path> {
        self.visible().get(self.selected).map(|r| r.path.as_path())
    }

    /// Path of the first markdown *file* in display order — used to pick what to
    /// open when no explicit focus file was given.
    pub fn first_file(&self) -> Option<PathBuf> {
        fn walk(node: &Node) -> Option<PathBuf> {
            for child in &node.children {
                if child.is_dir {
                    if let Some(p) = walk(child) {
                        return Some(p);
                    }
                } else {
                    return Some(child.path.clone());
                }
            }
            None
        }
        walk(&self.root)
    }

    /// Move the selection down/up by one visible row (saturating at the ends).
    pub fn select_next(&mut self) {
        let n = self.visible().len();
        if n > 0 {
            self.selected = (self.selected + 1).min(n - 1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Select a specific visible-row index (clamped). Used for mouse clicks.
    pub fn select_index(&mut self, idx: usize) {
        let n = self.visible().len();
        if n > 0 {
            self.selected = idx.min(n - 1);
        }
    }

    /// Map a screen row *within the sidebar viewport* (0-based, before scroll)
    /// to a visible-row index, if one exists there.
    pub fn row_at(&self, view_row: usize) -> Option<usize> {
        let idx = self.scroll + view_row;
        (idx < self.visible().len()).then_some(idx)
    }

    /// Activate the selected row. A directory toggles expand/collapse and
    /// returns `None`; a file returns its path for the caller to open.
    pub fn activate(&mut self) -> Option<PathBuf> {
        // Search results are always files — return the path directly, never the
        // tree's expand/collapse path (which would `rebuild_visible` and wipe
        // the results).
        if self.search.is_some() {
            return self.visible().get(self.selected).map(|r| r.path.clone());
        }
        let row = self.visible.get(self.selected)?;
        if row.is_dir {
            let path = row.path.clone();
            if self.expanded.contains(&path) {
                self.expanded.remove(&path);
            } else {
                self.expanded.insert(path);
            }
            let keep = self.selected;
            self.rebuild_visible();
            self.selected = keep.min(self.visible.len().saturating_sub(1));
            None
        } else {
            Some(row.path.clone())
        }
    }

    /// Adjust `scroll` so the selection sits within a `view_h`-row window.
    pub fn ensure_visible(&mut self, view_h: usize) {
        if view_h == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + view_h {
            self.scroll = self.selected + 1 - view_h;
        }
    }

    /// Rebuild [`visible`](Self::visible) by flattening the tree according to the
    /// current expansion set. The root node itself is not shown — its children
    /// form the top level.
    fn rebuild_visible(&mut self) {
        let mut out = Vec::new();
        Self::flatten(&self.root, 0, &self.expanded, &mut out);
        self.visible = out;
        if self.selected >= self.visible.len() {
            self.selected = self.visible.len().saturating_sub(1);
        }
    }

    fn flatten(node: &Node, depth: u16, expanded: &HashSet<PathBuf>, out: &mut Vec<VisibleRow>) {
        for child in &node.children {
            let is_expanded = child.is_dir && expanded.contains(&child.path);
            out.push(VisibleRow {
                depth,
                is_dir: child.is_dir,
                expanded: is_expanded,
                name: child.name.clone(),
                path: child.path.clone(),
                detail: None,
            });
            if is_expanded {
                Self::flatten(child, depth + 1, expanded, out);
            }
        }
    }

    /// Collect every file path in the tree, depth-first in display order.
    fn collect_files(node: &Node, out: &mut Vec<PathBuf>) {
        for child in &node.children {
            if child.is_dir {
                Self::collect_files(child, out);
            } else {
                out.push(child.path.clone());
            }
        }
    }

    /// Run a combined filename (fuzzy) + content (grep) search over the tree and
    /// switch the sidebar to a ranked flat result list. Case-insensitive (ASCII
    /// fold; non-ASCII isn't case-folded — acceptable for v1). Synchronous and
    /// bounded by `MAX_SEARCH_FILES` / `MAX_SEARCH_FILE_BYTES`.
    pub fn set_search(&mut self, query: &str) -> SearchOutcome {
        let q = query.trim().to_ascii_lowercase();
        self.selected = 0;
        self.scroll = 0;
        if q.is_empty() {
            self.search = Some(Vec::new());
            return SearchOutcome {
                matched: 0,
                truncated: false,
            };
        }

        let mut files: Vec<PathBuf> = Vec::new();
        Self::collect_files(&self.root, &mut files);
        let truncated = files.len() > MAX_SEARCH_FILES;
        files.truncate(MAX_SEARCH_FILES);

        let root = self.root.path.clone();
        struct Hit {
            row: VisibleRow,
            name_score: Option<i32>,
            count: usize,
        }
        let mut hits: Vec<Hit> = Vec::new();
        for path in files {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_ascii_lowercase();
            let name_score = fuzzy_score(&q, &rel);
            let count = count_in_file(&path, &q);
            if name_score.is_none() && count == 0 {
                continue;
            }

            // Dimmed locator: parent dir (relative to root) + match count.
            let mut detail = path
                .parent()
                .and_then(|p| p.strip_prefix(&root).ok())
                .map(|p| p.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_default();
            if count > 0 {
                if !detail.is_empty() {
                    detail.push_str(" · ");
                }
                detail.push_str(&format!(
                    "{count} match{}",
                    if count == 1 { "" } else { "es" }
                ));
            }
            let detail = (!detail.is_empty()).then(|| truncate_str(&detail, MAX_DETAIL_LEN));

            hits.push(Hit {
                row: VisibleRow {
                    depth: 0,
                    is_dir: false,
                    expanded: false,
                    name,
                    path,
                    detail,
                },
                name_score,
                count,
            });
        }

        // Filename hits (by fuzzy score desc) rank above content-only hits (by
        // match count desc); tie-break alphabetically.
        hits.sort_by(|a, b| {
            b.name_score
                .is_some()
                .cmp(&a.name_score.is_some())
                .then_with(|| {
                    b.name_score
                        .unwrap_or(i32::MIN)
                        .cmp(&a.name_score.unwrap_or(i32::MIN))
                })
                .then_with(|| b.count.cmp(&a.count))
                .then_with(|| {
                    a.row
                        .name
                        .to_ascii_lowercase()
                        .cmp(&b.row.name.to_ascii_lowercase())
                })
        });

        let rows: Vec<VisibleRow> = hits.into_iter().map(|h| h.row).collect();
        let matched = rows.len();
        self.search = Some(rows);
        SearchOutcome { matched, truncated }
    }

    /// Leave search mode and restore the tree. If `reveal` is given, its ancestor
    /// directories are expanded and it is selected (so an opened result stays
    /// visible); otherwise selection resets to the top.
    pub fn clear_search(&mut self, reveal: Option<&Path>) {
        self.search = None;
        if let Some(path) = reveal {
            self.expand_to(path);
        }
        self.rebuild_visible();
        self.selected = reveal
            .and_then(|p| self.visible.iter().position(|r| r.path == p))
            .unwrap_or(0);
        self.scroll = 0;
    }
}

/// Fuzzy-subsequence score of `query` within `hay` (both already lowercased).
/// `None` if `query` is not a subsequence of `hay`. Higher is better: contiguous
/// runs and matches right after a separator/word boundary score more; longer
/// haystacks are lightly penalized so shorter paths win ties.
fn fuzzy_score(query: &str, hay: &str) -> Option<i32> {
    if query.is_empty() {
        return None;
    }
    let mut q = query.chars().peekable();
    let qlen = query.chars().count();
    let mut score = 0i32;
    let mut matched = 0usize;
    let mut prev_matched = false;
    let mut prev: Option<char> = None;
    for c in hay.chars() {
        match q.peek() {
            Some(&want) if c == want => {
                score += 1;
                if prev_matched {
                    score += 3;
                }
                if prev.is_none_or(|p| matches!(p, '/' | '\\' | '_' | '-' | ' ' | '.')) {
                    score += 2;
                }
                matched += 1;
                prev_matched = true;
                q.next();
            }
            Some(_) => prev_matched = false,
            None => break,
        }
        prev = Some(c);
    }
    (matched == qlen).then_some(score - (hay.chars().count() as i32) / 16)
}

/// Count case-insensitive occurrences of `needle` (already lowercased) in the
/// file's contents. Returns 0 for unreadable, oversized, or non-matching files.
fn count_in_file(path: &Path, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_SEARCH_FILE_BYTES) {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
    count_substring(&text, needle)
}

/// Count non-overlapping occurrences of `needle` in `hay`.
fn count_substring(hay: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = hay[start..].find(needle) {
        count += 1;
        start += pos + needle.len();
    }
    count
}

/// Truncate to at most `max` chars (appending `…` if cut), replacing control
/// characters with spaces so a detail stays single-line.
fn truncate_str(s: &str, max: usize) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if cleaned.chars().count() <= max {
        return cleaned;
    }
    let mut out: String = cleaned.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temp dir under the OS temp root with a unique-ish name. No
    /// `Date::now`/random available in some harnesses, so derive uniqueness from
    /// the test thread name + an atomic counter.
    fn temp_root(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("mdview-ft-{tag}-{n}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"# x\n").unwrap();
    }

    #[test]
    fn prunes_dirs_without_markdown() {
        let root = temp_root("prune");
        touch(&root.join("a.md"));
        fs::create_dir_all(root.join("empty/nested")).unwrap();
        fs::write(root.join("empty/nested/notes.txt"), b"x").unwrap();
        touch(&root.join("docs/guide.md"));

        let tree = FileTree::build(&root, None).unwrap();
        let names: Vec<_> = tree.visible().iter().map(|r| r.name.as_str()).collect();
        // `empty/` is pruned (no markdown); `docs/` and `a.md` remain.
        assert!(names.contains(&"docs"), "docs dir should show: {names:?}");
        assert!(names.contains(&"a.md"), "a.md should show: {names:?}");
        assert!(!names.contains(&"empty"), "empty pruned: {names:?}");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn dirs_sort_before_files() {
        let root = temp_root("sort");
        touch(&root.join("zeta.md"));
        touch(&root.join("alpha.md"));
        touch(&root.join("subdir/inner.md"));

        let tree = FileTree::build(&root, None).unwrap();
        let names: Vec<_> = tree.visible().iter().map(|r| r.name.as_str()).collect();
        // Directory first, then files alphabetically.
        assert_eq!(
            names,
            vec!["subdir", "alpha.md", "zeta.md"],
            "got {names:?}"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn expand_collapse_via_activate() {
        let root = temp_root("expand");
        touch(&root.join("docs/guide.md"));
        touch(&root.join("top.md"));

        let mut tree = FileTree::build(&root, None).unwrap();
        // Collapsed by default: only "docs" and "top.md" at top level.
        assert_eq!(tree.visible().len(), 2);
        // Select the dir (it sorts first) and activate to expand.
        tree.select_index(0);
        assert!(tree.activate().is_none(), "activating a dir returns None");
        let names: Vec<_> = tree.visible().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["docs", "guide.md", "top.md"],
            "expanded: {names:?}"
        );
        // Collapse again.
        tree.select_index(0);
        tree.activate();
        assert_eq!(tree.visible().len(), 2, "collapsed again");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn focus_expands_ancestors_and_selects() {
        let root = temp_root("focus");
        let target = root.join("a/b/deep.md");
        touch(&target);
        touch(&root.join("other.md"));

        let tree = FileTree::build(&root, Some(&target)).unwrap();
        assert_eq!(
            tree.selected_path().and_then(|p| p.canonicalize().ok()),
            target.canonicalize().ok(),
            "focus file should be selected"
        );
        // Its ancestor dirs are expanded → deep.md is visible.
        assert!(tree.visible().iter().any(|r| r.name == "deep.md"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn activate_file_returns_path() {
        let root = temp_root("activate-file");
        touch(&root.join("only.md"));
        let mut tree = FileTree::build(&root, None).unwrap();
        tree.select_index(0);
        assert_eq!(
            tree.activate().as_deref(),
            Some(root.join("only.md").as_path())
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_tree_is_empty() {
        let root = temp_root("empty");
        fs::write(root.join("notes.txt"), b"x").unwrap();
        let tree = FileTree::build(&root, None).unwrap();
        assert!(tree.is_empty());
        assert!(tree.selected_path().is_none());
        assert!(tree.first_file().is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scroll_follows_selection() {
        let root = temp_root("scroll");
        for i in 0..20 {
            touch(&root.join(format!("file{i:02}.md")));
        }
        let mut tree = FileTree::build(&root, None).unwrap();
        for _ in 0..15 {
            tree.select_next();
        }
        tree.ensure_visible(5);
        assert!(tree.selected() >= tree.scroll());
        assert!(tree.selected() < tree.scroll() + 5);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn fuzzy_score_subsequence_and_ranking() {
        // Non-subsequence → None.
        assert!(fuzzy_score("xyz", "guide/api.md").is_none());
        // Subsequence → Some.
        assert!(fuzzy_score("api", "guide/api.md").is_some());
        // Contiguous + boundary match scores higher than scattered.
        let contiguous = fuzzy_score("api", "api.md").unwrap();
        let scattered = fuzzy_score("api", "a-p-i-x.md").unwrap();
        assert!(contiguous > scattered, "{contiguous} !> {scattered}");
    }

    #[test]
    fn count_substring_counts_occurrences() {
        assert_eq!(count_substring("a tuning of tuning", "tuning"), 2);
        assert_eq!(count_substring("nothing here", "xyz"), 0);
        assert_eq!(count_substring("aaaa", "aa"), 2); // non-overlapping
    }

    #[test]
    fn search_matches_filename_and_content() {
        let root = temp_root("search");
        fs::write(root.join("api.md"), b"# API\nendpoints\n").unwrap();
        fs::create_dir_all(root.join("guide")).unwrap();
        // No "api" in the name, but it appears in the body twice.
        fs::write(
            root.join("guide/intro.md"),
            b"the api is great; api again\n",
        )
        .unwrap();
        fs::write(root.join("unrelated.md"), b"nothing relevant here\n").unwrap();

        let mut tree = FileTree::build(&root, None).unwrap();
        let outcome = tree.set_search("api");
        assert!(tree.is_searching());
        assert_eq!(outcome.matched, 2, "api.md (name) + intro.md (content)");
        let names: Vec<_> = tree.visible().iter().map(|r| r.name.as_str()).collect();
        // Filename hit ranks above the content-only hit.
        assert_eq!(names, vec!["api.md", "intro.md"], "got {names:?}");
        // The content hit carries a match count in its detail.
        let intro = &tree.visible()[1];
        assert!(
            intro.detail.as_deref().unwrap_or("").contains("2 matches"),
            "detail: {:?}",
            intro.detail
        );
        // Activating a result returns its path (never toggles a tree dir).
        tree.select_index(1);
        assert_eq!(
            tree.activate().as_deref(),
            Some(root.join("guide/intro.md").as_path())
        );

        // Gibberish → no matches.
        let none = tree.set_search("zzzqqq");
        assert_eq!(none.matched, 0);
        assert!(tree.visible().is_empty());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn clear_search_restores_tree_and_reveals_file() {
        let root = temp_root("clear-search");
        let deep = root.join("guide/advanced/tuning.md");
        touch(&deep);
        touch(&root.join("top.md"));

        let mut tree = FileTree::build(&root, None).unwrap();
        // The deep file is in a collapsed subtree initially.
        assert!(!tree.visible().iter().any(|r| r.name == "tuning.md"));
        tree.set_search("tuning");
        assert!(tree.is_searching());

        // Clearing with the deep file as the reveal target expands its ancestors
        // and selects it.
        tree.clear_search(Some(&deep));
        assert!(!tree.is_searching());
        assert_eq!(tree.selected_path(), Some(deep.as_path()));
        assert!(tree.visible().iter().any(|r| r.name == "tuning.md"));

        fs::remove_dir_all(&root).ok();
    }
}
