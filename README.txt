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

Ideas taken elsewhere should be attributed to Andy Kurnia.


INITIAL SETUP

brew install rustup-init
rustup-init
(accept defaults)
(restart shell to activate .profile)

(cd src for convenience)
(put some word list .txt files and leaves.csv files in current directory)
(toggle "if false" in fn main in main.rs to compile leaves and word graphs)
cargo run --release

(toggle it back to false)


RUNNING

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
- bites is Kurnia Bites, a data structure used to store bytes.
- most of the rest are just data structures.


PERFORMANCE

- Minimize allocations. (Exception: When building data files.)
- Minimize indirections. Allocate one r*c vector, not r separate c vectors.
- Minimize unnecessary actions.
- Boxed slice takes two machine words (pointer, length).
- Vector takes three (capacity). This wastes space if it is read-only.
- Instead of full dynamic dispatch, use generics and enums.
- Conserve register usage, use 8/16/32-bit types instead of 64-bit.


DATA STRUCTURE

alphabet defines tile numbers. Tiles are numbered consecutively from 01. Tile
00 is reserved and may mean different things. By convention, 00 on rack means
blank. High bit is set when blank is played, so 81 means 01 played as a blank.
Tile labels are strings, so multi-codepoint labels are possible. This module
does not define how strings are parsed into tiles as that is a
non-deterministic process in the general case and is only useful when building
the word graph.

bites implements a value type that stores bytes. Assuming usize is 64 bits, a
Vec takes 24 bytes and heap, and a boxed slice takes 16 bytes and heap. Given
that a typical rack is just 7 bytes, storing 10000 moves takes too much space
and runtime overhead. Bytes is the same size as a boxed slice (16 bytes, for
alignment benefits) but stores small byte slices (up to 15 bytes) inline,
regaining cache locality and reserving heap usage only for longer slices.

board_layout encodes the board dimension and the premiums layout. Most squares
are face-value squares. This also decides where the star is.

game_config combines things like alphabet and board_layout.

matrix includes tools useful to work with a flattened 2-dimensional vector. Dim
encodes the rows and cols, and gives a way to access the indices for iterating
across a row or down a column. This is in the form of a Strider.
Across-direction striders multiply by one, but the ergonomy from unifying
across and down code justifies the runtime cost.

There are also ad-hoc data structures to represent things like board_tiles
(row-major, 00 is empty, 01-3f is tile, 81-bf is blank tile), cross set bits
(bit 0 reserved for adjacency flag), racks (a slice of 00-3f where 00 is blank,
always sorted), tallies (a slice of counters of the length of alphabet size),
exchange moves (same format as racks), place moves (01-3f is playing a tile,
81-bf is playing a blank tile, and 00 is playing through an on-board tile).


KURNIA WORD GRAPH

Conceptually this is a flat array of nodes (tile, accepts, is_end, arc_index).
Each entry is 32-bit. tile is 8 bits (subject to change). accepts and is_end
are 1 bit each. This is currently stored as little-endian. While bit shifting
is done on every access, this is offset by better cache line usage from a much
more compact memory footprint.

Tiles are numbered 01 to 3f consecutively. It is unknown if tile 3f works. Tile
00 is special. Tiles 40 to ff are not supported. For English alphabet, 01 to 1a
would represent A to Z, the rest would be invalid. It is guaranteed that the
tile is valid. This is called tile as it is a machine byte and is not required
to correspond to any alphabet label.

accepts encodes if adding the tile would complete the current word. The file
format cannot encode empty strings. In DFA (Deterministic Finite Automaton)
parlance, this encodes if the state is an accepting state.

is_end encodes if the node has a next_index. If the node is not at end, the
next_index is the immediate next index. The next index is taken when rejecting
the current tile. It is guaranteed that the next entry has a tile strictly
greater than the current one, enabling sorted iteration. It is guaranteed that
the last node always has is_end=true, enabling recursive iteration.

arc_index encodes the node's arc index. The arc index is taken when accepting
the current tile, which is not the last tile in the word. arc_index=0 encodes a
dead end, such nodes usually come with accepts=true.

Some nodes are special and guaranteed to exist at specific indices. Usually
these nodes have tile=00, accepts=false, is_end=true.

Other nodes may be rearranged in any way. It is guaranteed that this graph is
acyclic and there are no redundant nodes. Recursively iterating from any node
will always generate a unique, finite set of words in sorted order. Different
from other implementations of similar structures, it is allowed to share the
tail end of a node.

The guarantees make it possible to compute and memoize word counts for each
root node. This can be used to efficiently transform a word to an index number
and vice versa. To reduce the stack depth required to compute this, the counts
are performed over all indices in reverse order, which works because of the
guarantee that there are no redundant nodes. Counting takes linear time as the
count for each index is filled in just once. Each node generates one word from
accepting in addition to also generating all the words the arc_index and
implicit next_index would.

In the basic DawgOnly format, node 0 is a special node that points to the root
node of the DAWG (Directed Acyclic Word Graph).

The most common way to traverse the graph is to enter an arc_index and seek for
the node with a specific label, propagating failures if it does not exist. It
is necessary to not prematurely enter arc_index as the accepts flag is stored
before following arc_index. A convenience function is provided for this
purpose, and index 0 is intentionally reserved to avoid special-casing the
first tile.

In the Gaddawg format, node 0 is as above and node 1 is a special node that
points to the root node of the GADDAG. Tile 00 is used here to indicate the
direction switch from leftwards to rightwards, here represented as @ to
correspond with the byte representation (40 hex, where 41 hex is A). The word
CARE would be entered here as ERAC, RAC@E, AC@RE, C@ARE. Note the absence of
the @ in the case that does not require switching direction; the @ is
guaranteed to be followed by at least one tile. Note also that a GADDAG
necessarily contains all but the root node of the DAWG; the nodes after RAC@,
AC@, and C@ point to the DAWG after CAR, CA, and C. This makes including the
DAWG a negligible cost: it's just one additional root node where the A points
to the GADDAG's A@.

It is desirable to put GADDAG-only nodes after nodes that are used for DAWG,
and it is often possible to do so without sacrificing file size. Doing so
reduces cache miss when traversing the DAWG part and reduces the size required
for word counts, as word counts are typically only useful for the DAWG part.
However, GADDAG-only nodes with implicit next_index to a node used for DAWG
will necessarily be placed in the DAWG part.


BUILDING KURNIA WORD GRAPH

To build a DAWG, given a sorted, unique word list (where word in this context
means a sequence of tiles), the algorithm simulates typing each word in
succession and finally backspacing the last word, using only backspace. For
example if the words are TENSE, TENSES, TEST, TESTS it would type TENSE, then
type S, then backspace to TE and type ST, then type S, and then backspace over
everything. So at each iteration it would find the length of the common prefix
between the current and previous word, backspace to that point, and type the
rest of the current word.

This implies a stack of transitions for each character position; to minimize
allocations, all transitions for each position are encoded one after another in
a single vector, with a separate vector to remember the indices that separate
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

While GADDAG-only nodes are placed after nodes that are used for DAWG,
sometimes there are several GADDAG-only nodes with implicit next_index to a
node used for DAWG. No effort is currently made to choose the best such node
that minimizes the size of the DAWG region.

Below are implementation-specific structs found in the builder.

MyHasher is a simple, deterministic hasher. Rust comes with a randomized hasher
that is secure but slow.

Transition records a temporary transition (tile, accepts, arc_index), with
arc_index referring to an index in State (another temporary structure).

TransitionStack is a stack of transition, one for each position being
considered. The algorithm is written to only need to append items to the
topmost entry and to push/pop entries as a whole, so this is stored in a
contiguous layout instead of a vector of vector. The indices decide where the
transitions are separated.

State is a temporary structure of (tile, accepts, arc_index, next_index).
States are created as items are popped off the TransitionStack, so they end up
in reverse order. This order is optimal for creation but not optimal for
saving. To find these states quickly, the code uses a hashmap. The sink state
(the DFA rejection state) is guaranteed to exist at index 0.

StatesDefragger determines how each state maps to the final file. States other
than the sink state will be assigned a destination index. Once the head index
is assigned, all next_index are implicitly followed and written after it.

The defragger efficiently decides how to overlap these to collapse common tail
end of a node. To do this, it first finds out one prev_index for each index.
For shared nodes, it does not matter which prev_index is chosen. For index 0,
it does not matter because the sink state is not written.

IsEnd and Accepts simply encapsulate a bool. They are zero cost and only there
for the compiler to ensure the arguments are not in the wrong order.


KURNIA LEAVE VALUES

This file maps leaves to int16, which is scaled from float32 by 1/256. The
input is a two-column CSV (leave,value). The tiles in each leave must be
pre-sorted (blanks first). The format does not encode empty rack, and this is
fine as non-existent entries are conceptually mapped to 0.0.

The KLV stores a KWG (with a length prefix) and a raw int16 array (with a
length prefix). The KWG is a standard DawgOnly KWG with the additional
allowance to use 00 to represent the blank tile. The int16 array is sorted
based on the sorted order of the leaves file. Both the length prefixes and the
int16s are in little-endian. The length prefixes are also 32-bit for alignment,
this may not matter yet but the cost is negligible.

Because each entry is sorted, finding the correct entry takes linear time.
Effectively for a 7-tile rack there will be about 26 next_index and about 7
arc_index. The index returned corresponds to the index in the int16 array.


KURNIA MOVE GENERATOR

The code is concerned only with three types of moves: place, exchange, pass.
Exchanging zero tiles is recorded as pass. It is explicitly out of scope to
handle challenges, time adjustments, end-of-game bookkeeping, and so on. The
goal here is, given a board position (tiles and rack), what are the best moves.

Place moves are generated one-dimensionally over each (across and down) lane,
and this is based only on the tiles and the "cross sets" from the perpendicular
(down or across, respectively) lane.

CROSS SETS

The cross sets are computed one-dimensionally too. For each empty square before
or after a contiguous set of non-empty squares, the code looks at the word that
would be formed if a zero-valued tile is placed there. This word will include
both the contiguous non-empty squares before and after that empty square
(usually only one of these will apply). The face-value score of the tiles are
precomputed, as well as the bit set of tiles that would fit to make a valid
word, and the fact that this square is adjacent to a tile. The last two are
encoded together in a single bit set. Because tile 00 is reserved, bit 0 will
represent the fact that there is a tile before or after this empty square. Bits
1 to 63 will represent if tile 01 to tile 3f fits to make a valid word. Storing
this in a 64-bit variable limits the alphabet size to 63. Note that a cross set
bits of 0 means no restrictions, while a cross set bits of 1 (bit 0 set) means
nothing works because there is a perpendicular restriction.

Computing the cross sets efficiently entails traversing the lane in reverse
order (right-to-left or bottom-to-top), as this corresponds with the GADDAG
format. When reaching the end of a contiguous set of tiles, the code updates
the cross set just after (at the current GADDAG node) and just before
(following the @-node from the curreng GADDAG node), and the scores. When the
square after the next empty square is not empty, the next contiguous set of
tiles are traversed an additional time for each possible tile to check if the
GADDAG, and one final additional time to include the scores. This is
necessarily non-deterministic as the GADDAG does not store all possible
one-tile inserts. If the GADDAG traversal for the first set of contiguous tiles
fail, it is not necessary to have the correct score, but it is done anyway to
help debugging.

Due to the access pattern, cross sets are stored transposed. So the down cross
sets are stored by rows in a rows*cols array, and the across cross sets are
stored by column in a cols*rows array. When generating place moves, only the
relevant consecutive segment is referred to, for better cache locality. It is
possible to halve the memory requirements by generating the across sets and the
down sets at the same place, but not doing so may open up more opportunities
for reuse.

When the board is empty, the cross set on the star is set to the bitwise
negation of 1 to allow the game to begin. This is only done for one direction.

SEGMENTATION

To generate place moves efficiently without duplicating work, the moves are
generated for disjoint (anchor, leftmost, rightmost) triplets. Moves fit the
triplet if it includes a tile at the anchor square and is entirely contained
within leftmost (inclusive) to rightmost (exclusive).

Moves that involve a tile on board are anchored at the rightmost such tile (to
allow GADDAG early pruning), and may not touch the previous set of contiguous
tiles again (as those would duplicate). They will also need at least one fresh
tile being placed.

Moves that do not involve a tile on board are anchored at a cross set, as they
must touch a perpendicular tile (except for the starting move). They may not
touch an existing tile (by definition), they may not touch an already processed
cross set (as those would duplicate) and there must be space for at least two
tiles.

The code repeatedly finds the rightmost tile, generates moves that involve that
tile and those in the gap that do not involve that tile, and moves the
rightmost to exclude up to the one empty square after the current set of tiles.

PLAYING TILES

This part of the generator recursively places a tile. Different from other
implementations, each recursion iteratively gets past tiles already on board,
conserving stack space for when a tile is placed from the rack, bounding the
recursion depth to the number of tiles on rack. Only words of length two and
above are considered, it is expected that single-tile words are not accepted.

The code starts at play_left and goes into play_right when encountering the
direction switch marker in the GADDAG. A move can only be recorded after
iteratively exhausting the contiguous set of tiles, and only if the GADDAG
pointer is at an accepting state. In play_left, the move always ends at the
anchor square. In play_right, the move may not end at the anchor square as that
move would duplicate the one just recorded in play_left.

Ordinarily, single-tile moves are generated twice for each direction. To avoid
this, at least two tiles must have been played from the rack, or at least one
tile from the rack when the move is unique. For one direction, all moves are
unique; for the other, a move is unique if there was a tile placed from the
rack on a square with no cross sets, which implies that there is no adjacent
tile in the perpendicular direction that would generate the same move. In the
implementation language, true is 1 and false is 0, so the condition is simply
(num_played + is_unique >= 2).

So each iteration of play_left and play_right is concerned with completing a
word, recording if necessary, and trying to place one more tile. If there is a
cross set, only those tiles are accepted; if there is no cross set, all tiles
are allowed. Placing one more tile is driven by the arcs available at that
GADDAG position. For each valid tile that meet the cross set requirement, both
the actual tile and the blank are attempted if the rack has it. When the
play_left encounters the 00 tile, it triggers play_right.

While contemplating the word to be played, it is not necessary to allocate new
byte arrays and make copies. Instead, the code preallocates a max(rows, cols)
array of 00, and puts the tiles at the right places. For example, a 3-tile word
from index 7 would be at index 7, 8, 9 and not at index 0, 1, 2. This
arrangement obviates repeated insertions. Indices corresponding to non-empty
squares are never overwritten and will remain 00, so assignments only occur
when placing tiles.

When a move is actually found, a callback is called. This callback would
receive the corresponding slice of the work buffer. Unless the callback
allocates, generating moves does not allocate.

As each tile is played, several variables are kept up-to-date. The main_score
includes the face-value score of each tile on the board that is played through,
as well as the rack tile value (the score of the tile played from rack,
affected by the tile multiplier). The perpendicular_score will accumulate, only
for the positions where a tile is played on a cross set perpendicularly
adjacent to a tile, the face-value of those and the tile value, both multiplied
by the word multiplier on that particular square. The word_multiplier is the
product of all word multipliers the current word has encountered, to later be
applied on the main word. The GADDAG pointer points to the node before
following the arc, because the accepts flag is encoded before following the
arc_index. There are also the rack leave and the number of tiles played from
rack, which are shared copies rather than passed through parameters because the
changes are easy to undo.

When the word is finalized, the score is main_score * word_multiplier +
perpendicular_score + bingo bonus if applicable.

HASTY VALUATION ALGORITHM

The hasty valuation algorithm is invented by Cesar Del Solar and used by
HastyBot in Woogles.io.

When the bag is not empty, moves are valued as score + leave.

When the board is empty, it applies a penalty of 0.7 for every vowel placed
next to a premium square. This also applies to blanks designated as vowels.

When the bag is empty, moves that play out are valued as score plus a bonus of
twice the leftover tiles, which corresponds to standard endgame math. Moves
that do not play out are valued as score minus (10 plus twice the kept tiles).

GENERATING ONLY TOP N MOVES

Because there may be many moves but only a few are needed, it does not make
sense to allocate space for all of them, make copies, sort the whole thing, and
discard most of it.

This code uses a bounded min-heap, so that the nth best move is easily
available. After n moves are generated, when considering the next move, if it
is not strictly better than the nth best move, no copying occurs. If it is,
then the nth best move is popped out and the new move is inserted into the
heap. The library does not implement heappushpop or heapreplace, but the
observed speed is good enough. At the end, the heap's backing vector is
recovered and sorted in-place.


POSSIBLE FUTURE WORK

Moves on each lane are independent from one another, and dependent only on the
tiles on that lane and the cross set. Playing a 7-tile move affects only 9
lanes out of 30, so moves can be pregenerated by lane and reused. In addition,
for 7 of those, only one tile is added. This may enable precomputation.

By capping the number of tiles played, a more inclusive rack tally can be
passed to the move generator. In the extreme case, the whole unseen pool can be
passed to the move generator to get all the possible moves in the endgame.
