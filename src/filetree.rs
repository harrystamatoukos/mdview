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
}

/// The browsable tree plus selection and scroll state.
pub struct FileTree {
    root: Node,
    expanded: HashSet<PathBuf>,
    selected: usize,
    visible: Vec<VisibleRow>,
    /// Index of the first visible row currently shown (for list scrolling).
    scroll: usize,
}

/// Whether a path looks like a markdown file by extension.
fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
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
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
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
            scroll: 0,
        };

        // Expand the ancestor chain of the focus file so it's revealed. `focus`
        // and `root_dir` come from the same CLI argument, so they share a path
        // space — no canonicalization needed (and mixing canonical focus paths
        // with as-read tree paths would silently fail to match).
        if let Some(focus) = focus {
            let mut cur = focus.parent();
            while let Some(dir) = cur {
                tree.expanded.insert(dir.to_path_buf());
                if dir == tree.root.path {
                    break;
                }
                cur = dir.parent();
            }
        }

        tree.rebuild_visible();

        // Select the focus row if we can match it; else the first row.
        if let Some(focus) = focus
            && let Some(idx) = tree.visible.iter().position(|r| r.path == focus)
        {
            tree.selected = idx;
        }
        Ok(tree)
    }

    /// Whether the tree has no rows at all.
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// The visible rows currently on screen (full list; the caller windows it
    /// with [`scroll`](Self::scroll) and [`ensure_visible`](Self::ensure_visible)).
    pub fn visible(&self) -> &[VisibleRow] {
        &self.visible
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
        self.visible.get(self.selected).map(|r| r.path.as_path())
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
        if !self.visible.is_empty() {
            self.selected = (self.selected + 1).min(self.visible.len() - 1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Select a specific visible-row index (clamped). Used for mouse clicks.
    pub fn select_index(&mut self, idx: usize) {
        if !self.visible.is_empty() {
            self.selected = idx.min(self.visible.len() - 1);
        }
    }

    /// Map a screen row *within the sidebar viewport* (0-based, before scroll)
    /// to a visible-row index, if one exists there.
    pub fn row_at(&self, view_row: usize) -> Option<usize> {
        let idx = self.scroll + view_row;
        (idx < self.visible.len()).then_some(idx)
    }

    /// Activate the selected row. A directory toggles expand/collapse and
    /// returns `None`; a file returns its path for the caller to open.
    pub fn activate(&mut self) -> Option<PathBuf> {
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
            });
            if is_expanded {
                Self::flatten(child, depth + 1, expanded, out);
            }
        }
    }
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
        assert_eq!(names, vec!["subdir", "alpha.md", "zeta.md"], "got {names:?}");

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
        assert_eq!(names, vec!["docs", "guide.md", "top.md"], "expanded: {names:?}");
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
        assert_eq!(tree.activate().as_deref(), Some(root.join("only.md").as_path()));
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
}
