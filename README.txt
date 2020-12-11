PROJECT NAME

This project needs a better name.


LICENSE

Copyright (C) 2020 Andy Kurnia. All rights reserved.

This is NOT free software. Any contributions must be done only with the
understanding that Andy Kurnia has full rights to the entirety of the
repository.

This repository contains experimental stuffs. In the course of experimenting,
some ideas, code, or data from other sources may be used. Those may not come
with redistribution rights, and whoever interacts with this repository will
need to separately gain access to those.


QUICK START

cargo run --release


DEVELOPING

cargo clippy
cargo fmt


CODE STRUCTURE

Code quality is mediocre at best.

Most files are in src.

- main is the entry point, what it does is not defined.
- display collects display helpers.
- error provides a generic error value.
- kwg deal with the Kurnia Word Graph file.
- klv deal with the Kurnia Leave Values file.
- build implements building kwg.
- movegen generates moves using the Kurnia generator.
- most of the rest are just data structures.


PERFORMANCE

- Minimize allocations. (Exception: When building data files.)
- Minimize indirections.
- Minimize unnecessary actions.
- Boxed slice takes two machine words (pointer, length).
- Vector takes three (capacity). This wastes space if it is read-only.


KURNIA WORD GRAPH

Conceptually this is a flat array of nodes (tile, accepts, is_end, arc_index).
Each entry is 32-bit. tile is 8 bits (subject to change). accepts and is_end
are 1 bit each. This is currently stored as little-endian.

Tiles are numbered 01 to 3f consecutively. It is unknown if tile 3f works. Tile
00 is special. Tiles 40 to ff are not supported. For English alphabet, 01 to 1a
would represent A to Z, the rest would be invalid. It is guaranteed that the
tile is valid.

accepts encodes if adding the tile would complete the current word. The file
format cannot encode empty strings.

is_end encodes if the node has a next_index. If the node is not at end, the
next_index is the immediate next index. The next index is taken when rejecting
the current tile. It is guaranteed that the next entry has a tile strictly
greater than the current one, enabling sorted iteration. It is guaranteed that
the last node always has is_end=true, enabling recursive iteration.

arc_index encodes the node's arc index. The arc index is taken when accepting
the current tile, which is not the last tile in the word. arc_index=0 encodes a
dead end, such nodes usually come with accepts=true.

Some nodes are special and guaranteed to exist at specific indexes. Usually
these nodes have tile=00, accepts=false, is_end=true.

Other nodes may be rearranged in any way. It is guaranteed that the whole nodes
is acyclic. Different from other implementations of similar structures, it is
allowed to share the tail end of a node.

In the basic DawgOnly format, node 0 is a special node that points to the root
node of the DAWG.

In the Gaddawg format, node 0 is as above and node 1 is a special node that
points to the root node of the GADDAG. Tile 00 is used here to indicate the
direction switch from leftwards to rightwards. The word CARE would be entered
here as ERAC, RAC@E, AC@RE, C@ARE. Note the absence of the @ in the case that
does not require switching direction. Note also that a GADDAG necessarily
contains all but the root node of the DAWG; the nodes after RAC@, AC@, and C@
point to the DAWG after CAR, CA, and C. This makes including the DAWG a
negligible cost.

The guarantees make it possible to compute word counts. This can be used to
efficiently transform a word to an index number and vice versa. (Refer to
code.)


BUILDING KURNIA WORD GRAPH

(Refer to the code for details.)

To build a DAWG, given a sorted, unique word list (where word in this context
means a sequence of tiles), the algorithm pretends to type each word in
succession and finally backspacing the word, using only backspace. For example
if the words are TENSE, TENSES, TEST, TESTS it would type TENSE, then type S,
then backspace to TE and type ST, then type S, and then backspace over
everything.

This implies a stack of transitions for each character position; to minimize
allocations, all transitions for each position are encoded one after another in
a single vector, with a separate vector to remember the indexes that separate
the transitions. When turning TENSES into TEST, the third position (after TE)
gets a second transition (on S) in addition to the first one (to N). Hence
states are only finalized during the backspacing phase.

Once the DAWG is built, if desired, GADDAG can be built that would refer to the
same DAWG. This is significantly faster than building the whole GADDAG from
scratch, as the duplicate nodes would have been identified during the DAWG
phase.

The states then need to be defragmented to identify where in the file they
should end up, because only the arc_index (and not next_index) are encoded
explicitly.


KURNIA LEAVE VALUES

This maps leave values to float32. The input is a two-column CSV (leave,value).
The tiles in each leave must be pre-sorted (blanks first). The format does not
encode empty rack (which is fine).

The KLV stores a KWG (with a length prefix) and a raw float32 array (with a
length prefix). The KWG is a standard DawgOnly KWG with the additional
allowance to use 00 to represent the blank tile. The float32 array is sorted
based on the sorted order of the leaves file.

Because each entry is sorted, finding the correct entry takes linear time.
Effectively for a 7-tile rack there will be about 26 next_index and about 7
arc_index. The index returned corresponds to the index in the float32 array.


FINDING MOVES

Just read the code.
