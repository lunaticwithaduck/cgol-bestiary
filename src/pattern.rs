//! RLE parsing — the format every Life pattern on the internet ships in.
//!
//! ```text
//! #N Gosper glider gun
//! #O Bill Gosper, 1970
//! #C The first known gun.
//! x = 36, y = 9, rule = B3/S23
//! 24bo$22bobo$12b2o6b2o12b2o$...!
//! ```
//!
//! `b` is dead, `o` is alive, `$` ends a row, `!` ends the pattern, and a
//! leading integer is a run count.

/// A totalistic birth/survival rule, as a pair of bitmasks over neighbour
/// counts 0..=8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub born: u16,
    pub survive: u16,
}

impl Rule {
    pub const LIFE: Rule = Rule { born: 1 << 3, survive: 1 << 2 | 1 << 3 };

    /// Accepts every spelling that occurs in the LifeWiki archive:
    /// `B3/S23`, `b3/s23`, `S23/B3`, and the old survival-first `23/3`.
    /// A `:T100,100`-style bounded-grid suffix is ignored.
    pub fn parse(src: &str) -> Option<Rule> {
        let lower = src.trim().to_ascii_lowercase();
        let body = lower.split(':').next()?;
        let (first, second) = body.split_once('/')?;

        let mask = |t: &str| -> Option<u16> {
            let mut m = 0u16;
            for c in t.chars() {
                let d = c.to_digit(10)?;
                if d > 8 {
                    return None;
                }
                m |= 1 << d;
            }
            Some(m)
        };

        let (born, survive) = match (first.strip_prefix('b'), first.strip_prefix('s')) {
            (Some(b), _) => (mask(b)?, mask(second.strip_prefix('s')?)?),
            (_, Some(s)) => (mask(second.strip_prefix('b')?)?, mask(s)?),
            // No letters at all: the old notation is survival first.
            _ => (mask(second)?, mask(first)?),
        };
        Some(Rule { born, survive })
    }

    pub fn is_life(&self) -> bool {
        *self == Rule::LIFE
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pattern {
    pub width: usize,
    pub height: usize,
    /// Live cells as `(x, y)`, relative to the pattern's top-left corner.
    pub live: Vec<(usize, usize)>,
    /// `#N` — the pattern's name.
    pub name: Option<String>,
    /// `#O` — discoverer and date.
    pub author: Option<String>,
    /// `#C` / `#c` — free-text commentary.
    pub comments: Vec<String>,
    /// Parsed rule, if the file declared one that we understood.
    pub rule: Option<Rule>,
    /// The rule exactly as written, for reporting on files we reject.
    pub rule_str: Option<String>,
}

impl Pattern {
    /// A pattern with no declared rule is Conway's Life by convention.
    pub fn is_life(&self) -> bool {
        self.rule.map_or(self.rule_str.is_none(), |r| r.is_life())
    }

    pub fn parse_rle(src: &str) -> Result<Self, String> {
        let mut p = Pattern::default();
        let mut declared: Option<(usize, usize)> = None;
        let mut body = String::new();
        let mut seen_header = false;

        for line in src.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('#') {
                let (tag, text) = rest.split_at(rest.len().min(1));
                let text = text.trim().to_string();
                match tag {
                    "N" => p.name = Some(text).filter(|t| !t.is_empty()),
                    "O" => p.author = Some(text).filter(|t| !t.is_empty()),
                    "C" | "c" | "D" => {
                        if !text.is_empty() {
                            p.comments.push(text)
                        }
                    }
                    // `#r` is the pre-header way of declaring a rule.
                    "r" => {
                        p.rule = Rule::parse(&text);
                        p.rule_str = Some(text);
                    }
                    _ => {}
                }
                continue;
            }
            if !seen_header && line.starts_with(['x', 'X']) {
                let (dims, rule) = parse_header(line)?;
                declared = Some(dims);
                if let Some(r) = rule {
                    p.rule = Rule::parse(&r);
                    p.rule_str = Some(r);
                }
                seen_header = true;
                continue;
            }
            body.push_str(line);
        }

        let (mut x, mut y) = (0usize, 0usize);
        let mut run: Option<usize> = None;

        for ch in body.chars() {
            match ch {
                '0'..='9' => run = Some(run.unwrap_or(0) * 10 + (ch as usize - '0' as usize)),
                '$' => {
                    y += run.take().unwrap_or(1);
                    x = 0;
                }
                '!' => break,
                c if c.is_whitespace() => {}
                c => {
                    let n = run.take().unwrap_or(1);
                    // Anything that isn't dead ('b' or '.') is a live state.
                    // Multi-state rules use A-X; we treat them all as alive.
                    if !matches!(c, 'b' | 'B' | '.') {
                        if !c.is_ascii_alphabetic() {
                            return Err(format!("unexpected character {c:?} in RLE body"));
                        }
                        for i in 0..n {
                            p.live.push((x + i, y));
                        }
                    }
                    x += n;
                }
            }
        }

        // Trust the header when present: it is the only way to know the
        // bounding box of a pattern whose last rows or columns are empty.
        let (width, height) = declared.unwrap_or_else(|| {
            let w = p.live.iter().map(|&(x, _)| x + 1).max().unwrap_or(0);
            let h = p.live.iter().map(|&(_, y)| y + 1).max().unwrap_or(0);
            (w, h)
        });
        p.width = width.max(p.live.iter().map(|&(x, _)| x + 1).max().unwrap_or(0));
        p.height = height.max(p.live.iter().map(|&(_, y)| y + 1).max().unwrap_or(0));

        Ok(p)
    }
}

type Header = ((usize, usize), Option<String>);

fn parse_header(line: &str) -> Result<Header, String> {
    let (mut x, mut y, mut rule) = (None, None, None);
    for field in line.split(',') {
        let Some((key, val)) = field.split_once('=') else {
            continue;
        };
        match key.trim() {
            "x" | "X" => x = val.trim().parse().ok(),
            "y" | "Y" => y = val.trim().parse().ok(),
            "rule" | "Rule" => rule = Some(val.trim().to_string()),
            _ => {}
        }
    }
    match (x, y) {
        (Some(x), Some(y)) => Ok(((x, y), rule)),
        _ => Err(format!("malformed RLE header: {line:?}")),
    }
}

pub const GLIDER: &str = "x = 3, y = 3, rule = B3/S23\nbob$2bo$3o!";

pub const BLINKER: &str = "x = 3, y = 1, rule = B3/S23\n3o!";

pub const GOSPER_GLIDER_GUN: &str = "x = 36, y = 9, rule = B3/S23\n\
    24bo$22bobo$12b2o6b2o12b2o$11bo3bo4b2o12b2o$2o8bo5bo3b2o$\
    2o8bo3bob2o4bobo$10bo5bo7bo$11bo3bo$12b2o!";

/// R-pentomino: five cells that take 1103 generations to settle.
pub const R_PENTOMINO: &str = "x = 3, y = 3, rule = B3/S23\nb2o$2ob$bo!";

pub const ACORN: &str = "x = 7, y = 3, rule = B3/S23\nbo$3bo$2o2b3o!";
