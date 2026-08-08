# Vendored golback

Upstream: https://github.com/favalosdev/golback
Version: 0.3.4
Commit:  16fdd01327ead60c373a130357b1921e165828b4 (2026-04-15)
Licence: MIT — see LICENSE, © 2026 Fernando Avalos

Vendored rather than used from crates.io because the HashLife integration needs
access to the quadtree itself, and `Node`, `Universe::nodes` and `Universe::root`
are all private. Two things are impossible from outside the crate:

* **Viewport rendering.** Descending the tree and stopping at nodes smaller
  than a pixel costs O(visible nodes). The public alternatives are `to_coords`,
  which materialises every live cell, and `is_alive`, which is ~290ms per
  megapixel.
* **Loading without flattening.** `from_coords` takes a flat list of every
  coordinate, so `metapixel-parity64` needs 1.6GB of pairs to load a pattern
  its own file describes in 5,572 nodes.

The commit that adds this directory is upstream untouched, so `git log` on this
directory shows exactly what we changed and why.
