//! Macrocell parsing — Golly's format for patterns too large to write out.
//!
//! ```text
//! [M2] (golly 2.0)
//! #R B3/S23
//! $$$$$$$......*$        <- an 8x8 leaf; rows split on '$', '*' alive
//! 4 0 0 1 0              <- level, nw, ne, sw, se (indices, 0 = empty)
//! ```
//!
//! The file is a DAG of quadtree nodes, one per line, each referring only to
//! nodes already defined. The last one is the root. Because identical
//! subtrees are shared, a 30KB file can describe a universe with billions of
//! live cells — so everything here works on the DAG. Nothing ever expands a
//! node into individual cells, and population is summed structurally.

use crate::pattern::Rule;

/// Node levels above this would overflow the `u128` coordinate arithmetic.
const MAX_LEVEL: u32 = 120;

#[derive(Debug, Clone, Copy)]
enum Node {
    /// An 8x8 block, bit `y * 8 + x`. Always level 3.
    Leaf(u64),
    Branch { level: u32, nw: u32, ne: u32, sw: u32, se: u32 },
}

/// A read-only view of one DAG node, for rebuilding the tree elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeView {
    /// An 8x8 block, bit `y * 8 + x`, with `y` increasing **downward**.
    Leaf(u64),
    /// Children are at `level - 1`; index 0 means empty.
    Branch { level: u32, nw: u32, ne: u32, sw: u32, se: u32 },
}

/// Bounding box of live cells relative to a node's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BBox {
    pub min_x: u128,
    pub min_y: u128,
    pub max_x: u128,
    pub max_y: u128,
}

impl BBox {
    pub fn width(&self) -> u128 {
        self.max_x - self.min_x + 1
    }
    pub fn height(&self) -> u128 {
        self.max_y - self.min_y + 1
    }
    fn shifted(self, dx: u128, dy: u128) -> BBox {
        BBox {
            min_x: self.min_x + dx,
            min_y: self.min_y + dy,
            max_x: self.max_x + dx,
            max_y: self.max_y + dy,
        }
    }
    fn union(self, o: BBox) -> BBox {
        BBox {
            min_x: self.min_x.min(o.min_x),
            min_y: self.min_y.min(o.min_y),
            max_x: self.max_x.max(o.max_x),
            max_y: self.max_y.max(o.max_y),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Macrocell {
    pub rule: Option<Rule>,
    pub rule_str: Option<String>,
    pub name: Option<String>,
    pub comments: Vec<String>,
    /// The root spans `2^level` cells square.
    pub level: u32,
    pub population: u128,
    pub bbox: Option<BBox>,
    /// How many distinct quadtree nodes the file defines — a decent proxy for
    /// how much structure HashLife has to chew on.
    pub nodes: usize,
    dag: Vec<Node>,
    root: u32,
}

impl Macrocell {
    /// A file with no `#R` line is Conway's Life, same convention as RLE.
    pub fn is_life(&self) -> bool {
        self.rule.map_or(self.rule_str.is_none(), |r| r.is_life())
    }

    pub fn parse(src: &str) -> Result<Macrocell, String> {
        let mut nodes: Vec<Node> = vec![Node::Leaf(0)]; // index 0 is the empty node
        let mut rule_str: Option<String> = None;
        let mut name: Option<String> = None;
        let mut comments: Vec<String> = Vec::new();

        for (lineno, line) in src.lines().enumerate() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            let err = |m: String| format!("line {}: {m}", lineno + 1);

            match line.as_bytes()[0] {
                b'[' => {} // "[M2] (golly 2.0)"
                b'#' => {
                    let (tag, text) = line[1..].split_at(line[1..].len().min(1));
                    let text = text.trim().to_string();
                    match tag {
                        "R" => rule_str = Some(text),
                        "N" => name = Some(text).filter(|t| !t.is_empty()),
                        "C" | "D" if !text.is_empty() => comments.push(text),
                        _ => {}
                    }
                }
                b'.' | b'*' | b'$' => {
                    nodes.push(Node::Leaf(parse_leaf(line).map_err(err)?));
                }
                b'0'..=b'9' => {
                    let mut it = line.split_ascii_whitespace();
                    let mut next = |what: &str| -> Result<u32, String> {
                        it.next()
                            .ok_or_else(|| err(format!("branch is missing {what}")))?
                            .parse::<u32>()
                            .map_err(|e| err(format!("bad {what}: {e}")))
                    };
                    let level = next("level")?;
                    let (nw, ne, sw, se) =
                        (next("nw")?, next("ne")?, next("sw")?, next("se")?);

                    if level < 4 {
                        return Err(err(format!("branch level {level} must be at least 4")));
                    }
                    if level > MAX_LEVEL {
                        return Err(err(format!("level {level} is beyond what we can measure")));
                    }
                    let n = nodes.len() as u32;
                    for (child, what) in [(nw, "nw"), (ne, "ne"), (sw, "sw"), (se, "se")] {
                        if child >= n {
                            return Err(err(format!("{what} refers to node {child}, not yet defined")));
                        }
                        // Children must be exactly one level down. Nothing
                        // checked this before, and an engine rebuilding the tree
                        // from these nodes would assemble a malformed quadtree.
                        if child != 0 {
                            let child_level = match nodes[child as usize] {
                                Node::Leaf(_) => 3,
                                Node::Branch { level, .. } => level,
                            };
                            if child_level != level - 1 {
                                return Err(err(format!(
                                    "{what} is level {child_level}, but a level-{level} node needs \
                                     level-{} children",
                                    level - 1
                                )));
                            }
                        }
                    }
                    nodes.push(Node::Branch { level, nw, ne, sw, se });
                }
                c => return Err(err(format!("unexpected line starting with {:?}", c as char))),
            }
        }

        if nodes.len() < 2 {
            return Err("no nodes defined".into());
        }
        let root = nodes.len() as u32 - 1;
        let level = match nodes[root as usize] {
            Node::Leaf(_) => 3,
            Node::Branch { level, .. } => level,
        };

        let rule = rule_str.as_deref().and_then(Rule::parse);
        let mut pop_memo = vec![None; nodes.len()];
        let mut box_memo = vec![None; nodes.len()];

        Ok(Macrocell {
            rule,
            rule_str,
            name,
            comments,
            level,
            population: population_of(&nodes, root, &mut pop_memo),
            bbox: bbox_of(&nodes, root, &mut box_memo),
            nodes: nodes.len() - 1,
            dag: nodes,
            root,
        })
    }

    /// The DAG's root node index.
    pub fn root_id(&self) -> u32 {
        self.root
    }

    /// Read one node of the DAG. `None` for index 0, the shared empty node.
    ///
    /// This exists so an engine can rebuild the tree in its own representation
    /// without ever expanding cells — the only way to load a pattern whose
    /// population will not fit in memory.
    pub fn node(&self, id: u32) -> Option<NodeView> {
        if id == 0 || id as usize >= self.dag.len() {
            return None;
        }
        Some(match self.dag[id as usize] {
            Node::Leaf(bits) => NodeView::Leaf(bits),
            Node::Branch { level, nw, ne, sw, se } => {
                NodeView::Branch { level, nw, ne, sw, se }
            }
        })
    }

    /// Expand the DAG into individual live cells, positioned relative to the
    /// pattern's own bounding box so the result starts at `(0, 0)`.
    ///
    /// This is the one operation that costs [`population`](Self::population)
    /// rather than node count, and it is why the whole file format exists to
    /// avoid it — `metapixel-p216-gun` would yield 128 million pairs. Callers
    /// are expected to check the population first.
    ///
    /// Returns `None` if the bounding box does not fit in `i64`.
    pub fn live_cells(&self) -> Option<Vec<(i64, i64)>> {
        let b = self.bbox?;
        if b.max_x > i64::MAX as u128 || b.max_y > i64::MAX as u128 {
            return None;
        }
        let mut out = Vec::with_capacity(self.population as usize);
        emit(&self.dag, self.root, 0, 0, b.min_x, b.min_y, &mut out);
        Some(out)
    }
}

fn emit(
    nodes: &[Node],
    idx: u32,
    ox: u128,
    oy: u128,
    off_x: u128,
    off_y: u128,
    out: &mut Vec<(i64, i64)>,
) {
    if idx == 0 {
        return;
    }
    match nodes[idx as usize] {
        Node::Leaf(0) => {}
        Node::Leaf(bits) => {
            let mut w = bits;
            while w != 0 {
                let i = w.trailing_zeros() as u128;
                out.push((
                    (ox + i % 8 - off_x) as i64,
                    (oy + i / 8 - off_y) as i64,
                ));
                w &= w - 1;
            }
        }
        Node::Branch { level, nw, ne, sw, se } => {
            let half = 1u128 << (level - 1);
            for (c, dx, dy) in [(nw, 0, 0), (ne, half, 0), (sw, 0, half), (se, half, half)] {
                // Skipping empty children is what keeps this proportional to
                // the population rather than to the size of the universe.
                if c != 0 {
                    emit(nodes, c, ox + dx, oy + dy, off_x, off_y, out);
                }
            }
        }
    }
}

fn parse_leaf(line: &str) -> Result<u64, String> {
    let mut bits = 0u64;
    for (y, row) in line.split('$').enumerate() {
        if row.is_empty() {
            continue; // an empty row, or the trailing '$'
        }
        if y >= 8 {
            return Err(format!("leaf has a row at y={y}, but leaves are 8x8"));
        }
        for (x, c) in row.chars().enumerate() {
            if x >= 8 {
                return Err(format!("leaf row {y} is longer than 8 cells"));
            }
            match c {
                '*' => bits |= 1 << (y * 8 + x),
                '.' => {}
                c => return Err(format!("unexpected {c:?} in leaf")),
            }
        }
    }
    Ok(bits)
}

/// Summed structurally, so a node shared a million times is counted once and
/// multiplied — never walked a million times.
fn population_of(nodes: &[Node], idx: u32, memo: &mut Vec<Option<u128>>) -> u128 {
    if idx == 0 {
        return 0;
    }
    if let Some(p) = memo[idx as usize] {
        return p;
    }
    let p = match nodes[idx as usize] {
        Node::Leaf(bits) => bits.count_ones() as u128,
        Node::Branch { nw, ne, sw, se, .. } => [nw, ne, sw, se]
            .iter()
            .map(|&c| population_of(nodes, c, memo))
            .sum(),
    };
    memo[idx as usize] = Some(p);
    p
}

/// A node's bounding box is independent of where the node sits, which is what
/// makes memoising it across a shared DAG correct.
fn bbox_of(nodes: &[Node], idx: u32, memo: &mut Vec<Option<Option<BBox>>>) -> Option<BBox> {
    if idx == 0 {
        return None;
    }
    if let Some(b) = memo[idx as usize] {
        return b;
    }
    let b = match nodes[idx as usize] {
        Node::Leaf(0) => None,
        Node::Leaf(bits) => {
            let mut bb: Option<BBox> = None;
            let mut w = bits;
            while w != 0 {
                let i = w.trailing_zeros() as u128;
                let (x, y) = (i % 8, i / 8);
                let cell = BBox { min_x: x, min_y: y, max_x: x, max_y: y };
                bb = Some(bb.map_or(cell, |b: BBox| b.union(cell)));
                w &= w - 1;
            }
            bb
        }
        Node::Branch { level, nw, ne, sw, se } => {
            let half = 1u128 << (level - 1);
            [(nw, 0, 0), (ne, half, 0), (sw, 0, half), (se, half, half)]
                .into_iter()
                .filter_map(|(c, dx, dy)| bbox_of(nodes, c, memo).map(|b| b.shifted(dx, dy)))
                .reduce(BBox::union)
        }
    };
    memo[idx as usize] = Some(b);
    b
}
