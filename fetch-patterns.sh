#!/usr/bin/env bash
# Pull the pattern corpora into www/. Nothing fetched here is committed —
# see .gitignore.
#
# Two sources, because they carry different things:
#
#   RLE       — the LifeWiki collection, ~2,300 patterns. conwaylife.com sits
#               behind a Cloudflare challenge that blocks automated access, so
#               we use a mirror that snapshotted the same all.zip.
#   Macrocell — Golly's HashLife pattern set. These are the patterns too large
#               to express as RLE at all: metapixels, linear propagators, the
#               self-replicating Demonoid.
set -euo pipefail
cd "$(dirname "$0")"

RLE_REPO=https://github.com/thomasdunn/cellular-automata-patterns.git
MC_REPO=https://github.com/AlephAlpha/golly.git
RLE_SRC=.patterns-src
MC_SRC=.patterns-mc-src
RLE_DEST=www/patterns
MC_DEST=www/patterns-mc

# ---- RLE ----------------------------------------------------------------
if [ -d "$RLE_SRC/.git" ]; then
  echo "updating $RLE_SRC"
  git -C "$RLE_SRC" pull --ff-only --quiet
else
  echo "cloning $RLE_REPO"
  rm -rf "$RLE_SRC"
  git clone --depth 1 --quiet "$RLE_REPO" "$RLE_SRC"
fi

mkdir -p "$RLE_DEST"
cp "$RLE_SRC"/patterns/conwaylife/*.rle "$RLE_DEST"/
cp "$RLE_SRC"/patterns/conwaylife/_README_.txt "$RLE_DEST"/ 2>/dev/null || true

# ---- Macrocell ----------------------------------------------------------
# Golly is a whole application; a blobless sparse checkout of one directory
# keeps this to about a megabyte instead of cloning the lot.
if [ -d "$MC_SRC/.git" ]; then
  echo "updating $MC_SRC"
  git -C "$MC_SRC" pull --ff-only --quiet
else
  echo "cloning $MC_REPO (sparse: Patterns/HashLife)"
  rm -rf "$MC_SRC"
  git clone --depth 1 --filter=blob:none --sparse --quiet "$MC_REPO" "$MC_SRC"
  git -C "$MC_SRC" sparse-checkout set Patterns/HashLife --quiet 2>/dev/null \
    || git -C "$MC_SRC" sparse-checkout set Patterns/HashLife
fi

rm -rf "$MC_DEST"
mkdir -p "$MC_DEST"
# Flatten the category directories; the catalogue records provenance anyway.
find "$MC_SRC/Patterns/HashLife" -type f \( -name '*.mc' -o -name '*.mc.gz' \) \
  -exec cp {} "$MC_DEST"/ \;
# Decompressing here means the Rust side never needs a gzip dependency, which
# matters because the same crate compiles to wasm.
gzip -df "$MC_DEST"/*.mc.gz 2>/dev/null || true

echo
echo "$(ls "$RLE_DEST"/*.rle 2>/dev/null | wc -l | tr -d ' ') RLE patterns in $RLE_DEST"
echo "$(ls "$MC_DEST"/*.mc 2>/dev/null | wc -l | tr -d ' ') macrocell patterns in $MC_DEST"
echo
echo "Next:  cargo run --release --bin index"
