# golback

A high-performance Game of Life engine using the [HashLife](https://en.wikipedia.org/wiki/Hashlife) algorithm with quadtree compression and memoization.

## Overview

golback represents a Game of Life universe as a recursive quadtree. Each node covers a 2^k × 2^k region of the grid and caches its future state, allowing the simulation to skip over vast stretches of time and space in a single step. Identical subtrees are shared automatically, so sparse or repetitive patterns consume very little memory.

The implementation supports:

- **Arbitrary step sizes** via binary decomposition — advance by any number of generations, not just powers of two
- **Variable-speed HashLife** — larger universes advance more generations per iteration
- **Configurable rules** — birth and survival neighbor counts are configurable; RLE files with embedded rule headers are parsed automatically
- **Efficient population tracking** — alive cell counts are maintained structurally, requiring no full-grid scan

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
golback = "0.3.3"
```

## Quick Start

```rust
use golback::universe::Universe;

let mut universe = Universe::new();
universe.load("patterns/glider.rle".to_string())?;

// Advance exactly 100 generations
universe.advance(100);

println!("Generation: {}", universe.epochs());
println!("Population: {}", universe.population());

// Iterate over alive cells
for (x, y) in universe.to_coords() {
    println!("({}, {})", x, y);
}
```

## Core Concepts

### The Quadtree

The universe is a quadtree where each internal node has four children — northwestern (`a`), northeastern (`b`), southwestern (`c`), southeastern (`d`) — and stores the alive cell count for its entire region. Leaf nodes are either `DEAD` or `ALIVE`. Identical subtrees share the same `NodeId`, so the tree is a DAG in practice.

### Memoization

Three caches drive performance:

- **`join`** — maps `(a, b, c, d)` quadrant tuples to node IDs, ensuring structural sharing
- **`zero`** — caches empty nodes at each level so blank regions cost nothing
- **`successor`** — memoizes the future state of any node at any step size, which is the core of HashLife

### Step Sizes and `advance`

`advance(gens)` decomposes `gens` in binary and applies one centred `successor` pass per set bit, doubling the step level each time. This means arbitrary generation counts like 1, 7, or 100 are all handled correctly and efficiently — 7 = 4 + 2 + 1 requires three passes.

`hash_life()` skips the decomposition entirely and advances by 2^(k−2) generations in one shot, which is the maximum speed for a universe of level k.

### State Snapshots

`state()` returns the current root `NodeId`, which uniquely identifies the universe's configuration at that moment. Because identical subtrees share the same node ID, this is a zero-cost snapshot — no copying occurs. `set_state(id)` restores a previously saved ID, rewinding the universe to that configuration.

This makes cycle detection, rewinding, and branching straightforward:

```rust
let checkpoint = universe.state();

universe.advance(100);

if universe.state() == checkpoint {
    println!("Cycle detected — universe returned to a previous state");
}

// Or simply rewind
universe.set_state(checkpoint);
```

Note that `epochs()` is not affected by `set_state` — it tracks total generations elapsed, not the current universe configuration.

### Coordinate System

The origin `(0, 0)` is at the centre of the universe. Coordinates are signed `i64` pairs, so the grid extends symmetrically in all directions. The universe expands automatically when cells are added outside current bounds.

## API Reference

The full API is documented on [docs.rs](https://docs.rs/golback).

Key methods on `Universe`:

| Method               | Description                                           |
| -------------------- | ----------------------------------------------------- |
| `new()`              | Create an empty universe with B3/S23 rules            |
| `init(dim)`          | Initialise as an empty 2^dim × 2^dim grid             |
| `load(path)`         | Load an RLE file; parses embedded rule headers        |
| `from_coords(cells)` | Populate from an iterator of `(i64, i64)` coordinates |
| `advance(gens)`      | Advance exactly `gens` generations                    |
| `hash_life()`        | Advance 2^(k−2) generations at maximum speed          |
| `add(coord)`         | Set a cell alive, expanding the universe if needed    |
| `delete(coord)`      | Set a cell dead                                       |
| `toggle(coord)`      | Flip a cell's state                                   |
| `is_alive(coord)`    | Query whether a cell is currently alive               |
| `to_coords()`        | Return all alive cell coordinates                     |
| `population()`       | Return the alive cell count                           |
| `epochs()`           | Return total generations elapsed                      |
| `state()`            | Return the current root node ID                       |
| `set_state(id)`      | Restore a previously saved root node ID               |

## Rules

golback supports any outer-totalistic rule expressible as a birth/survival neighbour count list. The default is Conway's B3/S23. Rules can be set via an RLE file header or accessed directly:

```rust
let mut universe = Universe::new();
universe.load("patterns/day_and_night.rle".to_string())?; // parses B3678/S34678

println!("Birth on: {:?}", universe.b());
println!("Survive on: {:?}", universe.s());
```

## Performance Notes

- `FxHashMap` is used throughout for cache lookups on small integer keys, avoiding cryptographic hash overhead on the hot path
- The successor cache is keyed on `(NodeId, Option<u32>)`, so different step sizes for the same node are cached independently
- For maximum throughput on long runs, prefer `hash_life()` over repeated `advance(1)` calls
