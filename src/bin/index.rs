//! Build the pattern catalogue.
//!
//! Two corpora, handled differently:
//!
//!   RLE       — parsed, then *run* through [`cgol_bestiary::analyse`] to derive
//!               period, speed and class.
//!   Macrocell — parsed as a quadtree DAG and measured structurally. These
//!               patterns cannot be simulated by the bitmap engine at all, so
//!               they are catalogued and flagged for a HashLife backend.
//!
//! `cargo run --release --bin index -- [--patterns DIR] [--patterns-mc DIR]
//!                                     [--out FILE] [--max-gens N] [--limit N]`

use cgol_bestiary::analysis::{analyse, Analysis, Budget, Class};
use cgol_bestiary::macrocell::Macrocell;
use cgol_bestiary::pattern::Pattern;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

struct Args {
    patterns: PathBuf,
    patterns_mc: PathBuf,
    out: PathBuf,
    budget: Budget,
    /// Second, far more generous pass for patterns the first pass could not
    /// resolve. Most settle in a few hundred generations; a handful of
    /// methuselahs need thousands and a lot of elbow room, and paying for that
    /// on all 2,300 would be pure waste.
    escalate: Budget,
    /// Only escalate patterns with at most this many live cells — the big ones
    /// are expensive and are almost never the interesting unresolved cases.
    escalate_max_pop: usize,
    limit: Option<usize>,
    jobs: usize,
}

fn parse_args() -> Args {
    let mut a = Args {
        patterns: PathBuf::from("www/patterns"),
        patterns_mc: PathBuf::from("www/patterns-mc"),
        out: PathBuf::from("www/catalog.json"),
        budget: Budget::default(),
        escalate: Budget {
            max_generations: 6000,
            // A glider covers a cell every 4 generations, so ~1500 of margin
            // is what 6000 generations actually needs.
            margin: 1500,
            max_grid: 4096,
            ..Budget::default()
        },
        escalate_max_pop: 300,
        limit: None,
        jobs: std::thread::available_parallelism().map_or(4, |n| n.get()),
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let val = |i: usize| argv.get(i + 1).cloned().unwrap_or_default();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--patterns" => a.patterns = PathBuf::from(val(i)),
            "--patterns-mc" => a.patterns_mc = PathBuf::from(val(i)),
            "--out" => a.out = PathBuf::from(val(i)),
            "--max-gens" => a.budget.max_generations = val(i).parse().unwrap_or(3000),
            "--margin" => a.budget.margin = val(i).parse().unwrap_or(160),
            "--max-grid" => a.budget.max_grid = val(i).parse().unwrap_or(2048),
            "--limit" => a.limit = val(i).parse().ok(),
            "--jobs" => a.jobs = val(i).parse().unwrap_or(4).max(1),
            "--escalate-gens" => a.escalate.max_generations = val(i).parse().unwrap_or(6000),
            "--escalate-margin" => a.escalate.margin = val(i).parse().unwrap_or(1500),
            "--escalate-pop" => a.escalate_max_pop = val(i).parse().unwrap_or(300),
            "--no-escalate" => {
                a.escalate_max_pop = 0;
                i -= 1; // takes no value
            }
            other => {
                eprintln!("unknown argument {other:?}");
                std::process::exit(2);
            }
        }
        i += 2;
    }
    a
}

/// One catalogue row, from either corpus.
struct Entry {
    file: String,
    format: &'static str,
    /// Which backend can actually run this.
    engine: &'static str,
    name: Option<String>,
    author: Option<String>,
    comments: Vec<String>,
    w: u128,
    h: u128,
    pop: u128,
    /// Present for RLE only — macrocell patterns are not simulated.
    analysis: Option<Analysis>,
    /// Present for macrocell only: the root spans `2^level` cells.
    level: Option<u32>,
    nodes: Option<usize>,
}

impl Entry {
    fn category(&self) -> &'static str {
        match &self.analysis {
            Some(a) => a.category(),
            None => "macrocell",
        }
    }
    fn sort_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.file.clone()).to_lowercase()
    }
}

fn main() {
    let args = parse_args();
    let started = Instant::now();

    let mut entries = run_rle(&args);
    let mc = run_macrocell(&args);
    entries.extend(mc);
    entries.sort_by_key(|e| (e.sort_name(), e.file.clone()));

    let elapsed = started.elapsed();
    summarise(&entries);

    if let Some(parent) = args.out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = render_json(&entries, &args, elapsed.as_secs_f64());
    match std::fs::write(&args.out, &json) {
        Ok(()) => println!(
            "\nwrote {} ({:.1} KB) in {:.1}s",
            args.out.display(),
            json.len() as f64 / 1024.0,
            elapsed.as_secs_f64()
        ),
        Err(e) => {
            eprintln!("could not write {}: {e}", args.out.display());
            std::process::exit(1);
        }
    }
}

fn list(dir: &PathBuf, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

// ---------------------------------------------------------------- RLE ----

fn run_rle(args: &Args) -> Vec<Entry> {
    let mut files = list(&args.patterns, "rle");
    if files.is_empty() {
        eprintln!(
            "no RLE files in {} — run ./fetch-patterns.sh first",
            args.patterns.display()
        );
        std::process::exit(1);
    }
    if let Some(n) = args.limit {
        files.truncate(n);
    }
    println!("{} RLE files in {}", files.len(), args.patterns.display());

    let started = Instant::now();
    let chunks: Vec<&[PathBuf]> = files.chunks(files.len().div_ceil(args.jobs).max(1)).collect();
    let (mut entries, mut skipped, mut bad) = (Vec::new(), 0usize, Vec::new());

    let shared = args;
    std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| scope.spawn(move || process_rle(chunk, shared)))
            .collect();
        for h in handles {
            let (ok, n, b) = h.join().expect("worker panicked");
            entries.extend(ok);
            skipped += n;
            bad.extend(b);
        }
    });

    println!(
        "  analysed {} Life patterns in {:.1}s  ({skipped} other rules, {} unreadable)",
        entries.len(),
        started.elapsed().as_secs_f64(),
        bad.len()
    );
    for (f, why) in bad.iter().take(5) {
        println!("    skipped {f}: {why}");
    }
    entries
}

fn process_rle(files: &[PathBuf], args: &Args) -> (Vec<Entry>, usize, Vec<(String, String)>) {
    let (mut ok, mut skipped, mut bad) = (Vec::new(), 0, Vec::new());
    for path in files {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let Ok(raw) = std::fs::read(path) else {
            bad.push((name, "unreadable".into()));
            continue;
        };
        let pattern = match Pattern::parse_rle(&String::from_utf8_lossy(&raw)) {
            Ok(p) => p,
            Err(e) => {
                bad.push((name, e));
                continue;
            }
        };
        if !pattern.is_life() {
            skipped += 1;
            continue;
        }
        if pattern.live.is_empty() {
            bad.push((name, "no live cells".into()));
            continue;
        }

        let mut analysis = analyse(&pattern, args.budget);
        if analysis.class == Class::Unresolved && pattern.live.len() <= args.escalate_max_pop {
            // The retry ran longer with more room, so its numbers supersede
            // the first pass whether or not it reached a verdict.
            analysis = analyse(&pattern, args.escalate);
        }

        ok.push(Entry {
            file: name,
            format: "rle",
            engine: "bitmap",
            name: pattern.name.clone(),
            author: pattern.author.clone(),
            comments: pattern.comments.clone(),
            w: pattern.width as u128,
            h: pattern.height as u128,
            pop: analysis.initial_population as u128,
            analysis: Some(analysis),
            level: None,
            nodes: None,
        });
    }
    (ok, skipped, bad)
}

// ---------------------------------------------------------- macrocell ----

fn run_macrocell(args: &Args) -> Vec<Entry> {
    let files = list(&args.patterns_mc, "mc");
    if files.is_empty() {
        println!(
            "\nno macrocell files in {} — skipping (re-run ./fetch-patterns.sh to get them)",
            args.patterns_mc.display()
        );
        return Vec::new();
    }
    println!("\n{} macrocell files in {}", files.len(), args.patterns_mc.display());

    let (mut entries, mut skipped, mut bad) = (Vec::new(), Vec::new(), Vec::new());
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let Ok(raw) = std::fs::read(path) else {
            bad.push((name, "unreadable".to_string()));
            continue;
        };
        let mc = match Macrocell::parse(&String::from_utf8_lossy(&raw)) {
            Ok(m) => m,
            Err(e) => {
                bad.push((name, e));
                continue;
            }
        };
        if !mc.is_life() {
            skipped.push((name, mc.rule_str.clone().unwrap_or_default()));
            continue;
        }
        let (w, h) = mc.bbox.map_or((0, 0), |b| (b.width(), b.height()));
        entries.push(Entry {
            file: name,
            format: "macrocell",
            engine: "hashlife",
            name: mc.name.clone(),
            author: None,
            comments: mc.comments.clone(),
            w,
            h,
            pop: mc.population,
            analysis: None,
            level: Some(mc.level),
            nodes: Some(mc.nodes),
        });
    }

    println!(
        "  measured {} Life patterns  ({} other rules, {} unreadable)",
        entries.len(),
        skipped.len(),
        bad.len()
    );
    for (f, r) in skipped.iter() {
        println!("    other rule  {f}  ({r})");
    }
    for (f, why) in bad.iter() {
        println!("    FAILED      {f}: {why}");
    }

    let mut by_pop: Vec<&Entry> = entries.iter().collect();
    by_pop.sort_by_key(|e| std::cmp::Reverse(e.pop));
    println!("\n  largest macrocell patterns:");
    for e in by_pop.iter().take(8) {
        println!(
            "    {:>15} cells  2^{:<2} universe  {:>7} nodes  {}",
            group(e.pop),
            e.level.unwrap_or(0),
            e.nodes.unwrap_or(0),
            e.name.clone().unwrap_or_else(|| e.file.clone())
        );
    }
    entries
}

/// Thousands separators without pulling in a crate for it.
fn group(n: u128) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// -------------------------------------------------------------- output ----

fn summarise(entries: &[Entry]) {
    use std::collections::BTreeMap;
    let mut by_cat: BTreeMap<&str, usize> = BTreeMap::new();
    for e in entries {
        *by_cat.entry(e.category()).or_default() += 1;
    }
    println!("\nclassification:");
    for (cat, n) in &by_cat {
        println!("  {n:>5}  {cat}");
    }

    let mut periods: BTreeMap<u32, usize> = BTreeMap::new();
    let mut speeds: BTreeMap<String, usize> = BTreeMap::new();
    for a in entries.iter().filter_map(|e| e.analysis.as_ref()) {
        if matches!(a.class, Class::Oscillator { .. }) {
            *periods.entry(a.period().unwrap_or(0)).or_default() += 1;
        }
        if let Some(s) = a.speed() {
            *speeds.entry(s).or_default() += 1;
        }
    }
    let top: Vec<String> = periods.iter().take(12).map(|(p, n)| format!("p{p}×{n}")).collect();
    if !top.is_empty() {
        println!("\noscillator periods: {}", top.join("  "));
    }
    let mut sp: Vec<_> = speeds.into_iter().collect();
    sp.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    if !sp.is_empty() {
        println!(
            "spaceship speeds:   {}",
            sp.iter().take(8).map(|(s, n)| format!("{s}×{n}")).collect::<Vec<_>>().join("  ")
        );
    }
}

fn esc(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn opt_str(key: &str, v: &Option<String>, out: &mut String) {
    out.push_str(key);
    out.push(':');
    match v {
        Some(s) => esc(s, out),
        None => out.push_str("null"),
    }
    out.push(',');
}

fn opt_num(key: &str, v: Option<i64>, out: &mut String) {
    let _ = match v {
        Some(n) => write!(out, "{key}:{n},"),
        None => write!(out, "{key}:null,"),
    };
}

fn render_json(entries: &[Entry], args: &Args, secs: f64) -> String {
    let mut o = String::with_capacity(entries.len() * 260);
    o.push_str("{\n");
    o.push_str(
        "\"sources\":[\
         \"RLE: LifeWiki collection (conwaylife.com/patterns/all.zip), mirrored at \
         github.com/thomasdunn/cellular-automata-patterns\",\
         \"Macrocell: Golly's Patterns/HashLife, mirrored at github.com/AlephAlpha/golly\"],\n",
    );
    let _ = write!(
        o,
        "\"generatedInSeconds\":{:.1},\n\"budget\":{{\"maxGenerations\":{},\"margin\":{},\"maxGrid\":{}}},\n",
        secs, args.budget.max_generations, args.budget.margin, args.budget.max_grid
    );
    let _ = write!(o, "\"count\":{},\n\"patterns\":[\n", entries.len());

    for (i, e) in entries.iter().enumerate() {
        o.push('{');
        opt_str("\"file\"", &Some(e.file.clone()), &mut o);
        opt_str("\"format\"", &Some(e.format.to_string()), &mut o);
        opt_str("\"engine\"", &Some(e.engine.to_string()), &mut o);
        opt_str("\"name\"", &e.name, &mut o);
        opt_str("\"author\"", &e.author, &mut o);

        o.push_str("\"comments\":[");
        for (j, c) in e.comments.iter().take(6).enumerate() {
            if j > 0 {
                o.push(',');
            }
            esc(c, &mut o);
        }
        o.push_str("],");

        // Macrocell bounding boxes can exceed 2^53; JS will read these as
        // doubles, which is fine for display but not for arithmetic.
        let _ = write!(o, "\"w\":{},\"h\":{},\"pop\":{},", e.w, e.h, e.pop);
        opt_str("\"category\"", &Some(e.category().to_string()), &mut o);
        opt_num("\"level\"", e.level.map(|l| l as i64), &mut o);
        opt_num("\"nodes\"", e.nodes.map(|n| n as i64), &mut o);

        match &e.analysis {
            Some(a) => {
                opt_num("\"period\"", a.period().map(|p| p as i64), &mut o);
                opt_str("\"speed\"", &a.speed(), &mut o);
                let settles = match a.class {
                    Class::Settles { at, .. } | Class::Stabilises { at, .. } => Some(at as i64),
                    _ => None,
                };
                let dies = match a.class {
                    Class::Dies { at } => Some(at as i64),
                    _ => None,
                };
                opt_num("\"settlesAt\"", settles, &mut o);
                opt_num("\"diesAt\"", dies, &mut o);
                let _ = write!(
                    o,
                    "\"gens\":{},\"finalPop\":{},\"maxPop\":{},\"reachedEdge\":{}",
                    a.generations, a.final_population, a.max_population, a.reached_edge
                );
            }
            None => {
                o.push_str(
                    "\"period\":null,\"speed\":null,\"settlesAt\":null,\"diesAt\":null,\
                     \"gens\":0,\"finalPop\":0,\"maxPop\":0,\"reachedEdge\":false",
                );
            }
        }
        o.push('}');
        if i + 1 < entries.len() {
            o.push(',');
        }
        o.push('\n');
    }
    o.push_str("]}\n");
    o
}
