# conway

A browser and showcase stage for the LifeWiki pattern collection, on top of a
bit-parallel Game of Life engine written in Rust and compiled to WebAssembly.

Every property in the catalogue — period, speed, class, the generation a
methuselah settles on — is **measured by running the pattern**, not parsed out
of its comments.

```sh
./fetch-patterns.sh                     # pull both corpora (not committed)
cargo run --release --bin index         # measure everything, write catalog.json
./build.sh                              # compile the wasm module
python3 -m http.server 8080 --directory www
```

## Two corpora

| | count | source | engine | how it's measured |
|---|---|---|---|---|
| **RLE** | 2,316 | LifeWiki collection | bit-parallel bitmap | simulated — period, speed, class derived by running it |
| **Macrocell** | 25 | Golly's `Patterns/HashLife` | HashLife | measured structurally from the quadtree DAG |

Macrocell is Golly's format for patterns too large to write out at all.
`metapixel-p216-gun.mc` describes **128 million live cells in 30KB** — because
identical subtrees are shared, the whole universe is 6,458 quadtree nodes.
`src/macrocell.rs` parses that DAG and sums population and bounding box
*structurally*, memoising per node; nothing is ever expanded into cells. A node
used a million times is counted once and multiplied.

These cannot be run by the bitmap engine at all, so they run on HashLife —
see below.

## Two engines

The bitmap engine is unbeatable on small dense patterns and useless on a
universe 2^25 cells square. HashLife is the reverse. The catalogue records
which backend each pattern needs and the stage switches between them.

HashLife comes from [`golback`](https://github.com/favalosdev/golback) (MIT)
rather than being hand-written. The alternative, `gol_engines`, implements
StreamLife and is the better algorithm — but it depends unconditionally on
tokio with `rt-multi-thread`, which **does not compile to
`wasm32-unknown-unknown`**. It can be made to, by marking tokio optional and
putting `mod quadtree_async` behind a feature (tokio is confined to three
files), but that means carrying a patch. `golback` compiles to wasm untouched.

Two properties of its API shape `src/hashlife.rs`:

- **Both the way in and the way out are flat cell lists.** `from_coords` wants
  every coordinate and `to_coords` returns every coordinate, so both cost
  population rather than node count. Hence a ceiling of 8 million cells:
  `metapixel-parity64` would need **1.6GB** just for the input vector, and 14.6
  seconds to build. 23 of the 25 patterns fit; the two 100M-cell metapixels are
  refused with an explanation.
- **`is_alive` is far too slow to render with** — about 290ms per megapixel,
  roughly 7fps on our viewport. So rendering rasterises a cached cell list
  instead: O(pixels) to clear plus O(population) to plot, refreshed only when
  the universe actually advances.

**The universe does not grow itself.** This one bit hard. Sized to the
pattern's bounding box plus a little, a glider travels 128 cells and then
decays against the wall into a 2×2 block — silently, with no error. An
undersized Gosper gun reports 564 cells where the truth is 11,144. Since empty
quadtree nodes are shared, an oversized universe costs almost nothing, so
patterns now get a minimum of 2^22 and six levels of headroom. Drift is also
tracked, and stepping *stops* once the pattern could be losing cells off the
edge — the same principle the analyser uses, for the same reason: an artefact
of the box is worse than nothing.

The payoff is `demonoid-c512-hashlife-friendly.mc`, a self-replicating
spaceship. Advance it 2,097,152 generations and it has built a complete copy of
itself, displaced exactly 4096 cells diagonally, at identical population —
precisely what the pattern's own header documents. That is 2 million
generations across a 272,449 × 268,312 region; for the bitmap engine it is not
a slow computation but an impossible one.

## The engine

Cells are packed 64 to a `u64` and the rule is evaluated with nothing but
shifts, `AND`, `OR` and `XOR` — 64 cells per instruction sequence, no branches
anywhere in the hot loop.

The Life rule needs a neighbour *count*, but a bit only holds 0 or 1. The trick
is to store counts as **bit-planes**: a value in `0..=3` for 64 columns lives in
two words, one per bit position. Addition then becomes a logic circuit, and each
bitwise op runs 64 copies of that circuit at once.

**1. Horizontal windows.** For a row word `r`, `r << 1` is the left neighbour of
every column and `r >> 1` is the right. A full adder over the three gives a
2-bit sum of each 3-cell window:

```rust
let x = lft ^ cur;
(x ^ rgt, (lft & cur) | (rgt & x))
```

**2. Stack three rows.** Add the three 2-bit numbers, column by column, into a
4-bit total. Call it `S9` — every cell in the 3×3 box, *centre included*.

**3. The rule.** Counting the centre is what makes this collapse. A live cell
needs 2 or 3 neighbours, so `S9 ∈ {3, 4}`; a dead cell needs exactly 3, so
`S9 = 3`. The whole of Life is:

```rust
next = (S9 == 3) | (mid & (S9 == 4))
```

and equality on bit-planes is just ANDs of bits and their complements.

About 45 instructions per 64 cells — under one per cell, against roughly 15–20
per cell for the obvious implementation. Measured with `cargo run --release
--bin bench` on an M-series Mac:

```
naive   512×512  ×20      0.041s      129 M cell-updates/sec
bitwise 512×512  ×200     0.004s   12,487 M cell-updates/sec
bitwise 2048×2048 ×100    0.022s   19,187 M cell-updates/sec
```

Treat the ~150× with some suspicion: the naive baseline uses `%` for wrapping,
which is genuinely awful, and LLVM is very likely auto-vectorising the `u64`
loop into NEON. Against a tuned scalar version, expect closer to 30–50×.

The one gotcha is that `<<` drops the bit at the word edge, so column 0's left
neighbour has to be pulled out of the previous word:

```rust
let lft = (cur << 1) | (prev >> 63);
```

On a torus the row's last word *is* the previous word, so wrapping comes out for
free. Grids can also be [`Boundary::Dead`], where everything off-grid is
permanently dead — necessary for the showcase, because on a torus a gun's own
gliders come back around and demolish it.

## Measuring what a pattern is

`src/analysis.rs` fingerprints every generation with a translation-invariant
hash of the live cells. When a fingerprint recurs, the pattern has entered a
cycle, and the displacement between the two occurrences says which kind:

| displacement | first seen | verdict |
|---|---|---|
| none, period 1 | gen 0 | still life |
| none | gen 0 | oscillator |
| nonzero | gen 0 | spaceship — speed from displacement ÷ period |
| any | later | settles after a transient |

Two things that are easy to get wrong:

**Escaping gliders defeat configuration hashing.** Acorn's debris settles at
generation 5206, but six gliders fly away forever, so the *configuration* never
repeats and no amount of extra budget will find a cycle. What conventionally
counts as "stabilising" is the **population** becoming periodic, which is a
separate and much cheaper check. Adding it moved 228 patterns out of
`unresolved` and made the whole index run three times faster, because far fewer
patterns escalate to the expensive second pass.

**A dead boundary invalidates verdicts.** Simulation stops the moment a live
cell reaches within one of the edge, since past that point a birth could happen
off-grid and we would be classifying an artefact of the box rather than Life on
the infinite plane. Patterns that hit the edge while still growing are recorded
as infinite growth; the rest are honestly reported as `unresolved`.

Of 2,316 Conway patterns: 94% get a verdict, 134 remain unresolved, and 26 are
larger than the analyser's grid cap (the biggest is 3.1M × 2.7M cells).

## Testing

Bit tricks fail in ways that only surface hundreds of generations later, or
only at a word boundary. `tests/differential.rs` diffs the fast engine against
the naive one **every single generation**, across single-word rows, multi-word
rows, sparse and dense soups, and short grids.

`tests/analysis.rs` pins the classifier to published figures: the glider is
`c/4 diagonal`, the LWSS is `c/2 orthogonal`, the R-pentomino stabilises at
generation **1103** and Acorn at **5206**.

`tests/hashlife.rs` holds the third-party engine against the one we already
trust — same soup, cell for cell, every generation for 80 generations across
three densities. It also checks that one jump of 512 generations lands exactly
where 512 single steps do, which is what caught the universe-sizing bug, and
that macrocell populations survive the round trip from our structural count
through enumeration and tree rebuilding.

And the index is itself the largest test there is — computing the right period
for two thousand independently documented patterns is strong evidence the bit
arithmetic is correct.

```sh
cargo test --release
```

## Layout

| | |
|---|---|
| `src/bitgrid.rs` | the engine |
| `src/naive.rs` | one byte per cell — the test oracle |
| `src/analysis.rs` | classification by simulation |
| `src/pattern.rs` | RLE parsing, rule normalisation |
| `src/macrocell.rs` | quadtree DAG parsing, structural measurement |
| `src/hashlife.rs` | HashLife backend for the macrocell patterns |
| `src/ffi.rs` | raw C ABI for the browser (no wasm-bindgen) |
| `src/bin/index.rs` | builds `catalog.json` |
| `src/bin/bench.rs` | naive vs. bitwise throughput |
| `www/` | pattern browser and stage |

## Patterns and attribution

Neither corpus is committed — `fetch-patterns.sh` pulls both, `.gitignore`
excludes them.

- **RLE** from [thomasdunn/cellular-automata-patterns](https://github.com/thomasdunn/cellular-automata-patterns),
  which snapshotted `conwaylife.com/patterns/all.zip`. conwaylife.com itself
  sits behind a Cloudflare challenge that blocks automated access, so it should
  not be scraped directly.
- **Macrocell** from Golly's `Patterns/HashLife`, via a blobless sparse
  checkout of [AlephAlpha/golly](https://github.com/AlephAlpha/golly) — about a
  megabyte instead of cloning a whole application. The script gunzips the
  `.mc.gz` files so the Rust side never needs a gzip dependency, which matters
  because the same crate compiles to wasm.

Pattern files carry their original `#N`/`#O`/`#C` attribution and are the work
of the Life community; LifeWiki content is CC BY-SA 3.0. Carry that attribution
if you redistribute them.

Rules are normalised and filtered in both formats. RLE uses four spellings
(`B3/S23`, `b3/s23`, the old survival-first `23/3`, `S23/B3`) and 65 files are
other rules — HighLife, Life without Death, Maze. Four of the macrocell files
are the `fangtian` metacells, which run a non-totalistic rule
(`B3-jknr4ity5ijk6i8/S23-a4city6c7c`) and are rejected.

**0E0P is on none of these mirrors.** It appears to be LifeWiki-only, and it is
also the worst possible first target at 93 million cells — the reference
benchmark for it runs StreamLife across eight worker threads, which is exactly
what wasm cannot do. The Demonoid gives the same self-replication payoff at
106,639 cells.

## Not done yet

- **Rolling row sums.** Each row's horizontal window sum is computed three
  times — as the row above, the middle, and the row below. A two-row rolling
  window should cut roughly a third of the work.
- **SIMD.** `v128` doubles the lane count, but wasm's vector shifts are
  *per-lane*, so carrying a bit across the lane boundary needs a shuffle.
- **Quadtree-native rendering and loading.** Both ends of the HashLife
  integration currently go through flat cell lists, which is what imposes the
  8-million-cell ceiling. Descending the quadtree straight into the viewport —
  pruning empty and sub-pixel nodes — and building the tree directly from our
  macrocell DAG would remove it entirely and let the 128M-cell metapixels run.
  Both need node-level access that `golback` does not expose, so it means
  forking it (MIT, ~1,000 lines) or writing our own engine.
- **The oversized RLE patterns.** 26 of them are still catalogued as
  `too-large`; routing those through HashLife too is mostly plumbing.
- **Thumbnails** in the browser list, rendered by the indexer.
