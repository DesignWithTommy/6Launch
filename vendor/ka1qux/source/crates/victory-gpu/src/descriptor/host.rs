//! Host descriptor heaps behind the title's shader-visible heaps (decision
//! 0007, step 3). Translated shaders index the heaps directly at the
//! title's descriptor positions, so each host heap keeps the title heap's
//! order: slot `i` is made from what the title wrote at slot `i`, when it
//! writes it (view creation and descriptor copies) and again when a draw
//! reads it through a table and finds the slot changed.

use std::sync::Mutex;

use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use super::{VIEW_TAG, View, view_at};
use crate::host::host;

pub(crate) struct BufferRanges<T> {
    passes: u64,
    pub(crate) by_slot: std::collections::HashMap<u32, (T, u64, u64)>,
}

/// The directly indexed buffer views of one heap, with the host copies
/// they retain between passes. Dropping the heap releases those copies.
pub(crate) struct BufferViews<T> {
    slots: Mutex<std::collections::BTreeSet<u32>>,
    changed: Mutex<std::collections::HashSet<u32>>,
    ranges: Mutex<BufferRanges<T>>,
    /// Ranges of slots made elsewhere (at the mirroring of copied records,
    /// [`sync_ranges`]) for the next pass to take as they are, without a
    /// look at the slot: by slot, the last one given.
    taken: Mutex<std::collections::HashMap<u32, (T, u64, u64)>>,
}

impl<T> BufferViews<T> {
    fn new() -> Self {
        Self {
            slots: Mutex::default(),
            changed: Mutex::default(),
            ranges: Mutex::new(BufferRanges {
                passes: 0,
                by_slot: Default::default(),
            }),
            taken: Mutex::default(),
        }
    }

    /// Slot `index` was made elsewhere and reads `range` (copy, from, to):
    /// the next pass takes it without looking at the slot.
    fn take_range(&self, index: u32, range: (T, u64, u64)) {
        self.taken.lock().unwrap_or_else(|e| e.into_inner()).insert(index, range);
    }

    fn written(&self, index: u32, buffer: bool) {
        if self.noted(index, buffer) {
            self.changed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(index);
        }
    }

    /// [`BufferViews::written`] for a slot the pass over buffer views is
    /// making itself ([`in_heap_pass`]): it takes the slot's range at
    /// once, so the slot is not looked at again by the next pass. True if
    /// the slot is (or was) one of the buffer views.
    fn noted(&self, index: u32, buffer: bool) -> bool {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        if buffer {
            slots.insert(index);
            true
        } else {
            slots.remove(&index)
        }
    }

    pub(crate) fn begin_pass(&self) -> (Vec<u32>, std::sync::MutexGuard<'_, BufferRanges<T>>) {
        let mut kept = self.ranges.lock().unwrap_or_else(|e| e.into_inner());
        let changed = std::mem::take(&mut *self.changed.lock().unwrap_or_else(|e| e.into_inner()));
        // A view may go stale without a write. Revisit all slots in a
        // fresh heap and every 16 passes thereafter: with [`rolling_rescan`]
        // a sixteenth of them at each pass.
        let slots = if kept.passes > 0 && rolling_rescan() {
            let phase = (kept.passes % 16) as u32;
            let all = self.slots.lock().unwrap_or_else(|e| e.into_inner());
            // What the share's slots no longer are goes, as the whole set
            // went every 16 passes.
            kept.by_slot.retain(|&index, _| index % 16 != phase || all.contains(&index));
            let mut slots: std::collections::HashSet<u32> = changed;
            slots.extend(all.iter().copied().filter(|&index| index % 16 == phase));
            slots.into_iter().collect()
        } else if kept.passes.is_multiple_of(16) {
            kept.by_slot.clear();
            self.slots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .copied()
                .collect()
        } else {
            changed.into_iter().collect()
        };
        // The ranges given since the last pass, before the slots it looks
        // at (a look at one of those takes what the slot shows now).
        let taken = std::mem::take(&mut *self.taken.lock().unwrap_or_else(|e| e.into_inner()));
        kept.by_slot.extend(taken);
        kept.passes = kept.passes.wrapping_add(1);
        (slots, kept)
    }
}

pub(crate) struct HostHeap {
    pub(crate) heap: ID3D12DescriptorHeap,
    cpu: usize,
    /// The shader-visible handle of the heap's start, asked once: a draw
    /// asks for slots' handles some 10,000 times a frame.
    gpu: u64,
    step: u32,
    /// Slots mirroring the title's; view heaps have `dynamic` more.
    count: u32,
    /// Dynamic slots past `count`: [`DYNAMIC`] in view heaps, none in
    /// sampler heaps. (Upstream falls back to 2^16 on a capped host heap;
    /// ours keeps 2^18 there: 2^16 and 2^17 both ran out in this title.)
    dynamic: u32,
    sampler: bool,
    /// The record (view index and generation) each slot was last made from.
    made: Mutex<Vec<u64>>,
    /// `made` as atomics, for [`HostHeap::mirror`] to find a slot made from
    /// the record it holds without taking `made`'s lock (nearly every
    /// mirroring: some 50,000 a frame). Written only under `made`'s lock,
    /// wherever `made` is: set once the slot's descriptor and what it keeps
    /// are in place, cleared before the slot is made anew or null.
    made_fast: Vec<std::sync::atomic::AtomicU64>,
    /// Per slot, a fingerprint of the buffer view its host descriptor was
    /// last made from (the view and the title memory it reads; 0: none),
    /// for a record that names the same view again ([`same_views`]). Set
    /// after the slot is made, cleared whenever the slot's host resource
    /// changes ([`HostHeap::keep`]); taken after `made`. Made on first use.
    shown: Mutex<Vec<u64>>,
    /// A fingerprint of the record each slot held when the pass over
    /// records copied in as bytes last looked at it
    /// ([`HostHeap::mirror_copied`]; 0: never looked at). Made on first use.
    copied: Mutex<Vec<u64>>,
    /// The host resource each slot views, held while the slot does: shaders
    /// index heaps directly, so a slot the title has not rewritten since it
    /// released the resource may still be read, and a freed host resource
    /// read there is a GPU page fault (the console reads stale memory).
    /// In pieces of [`KEPT_PIECE`] slots, each behind a lock of its own:
    /// the recording threads make their tables' slots at the same time
    /// (after victory-fork's pull request #1146).
    kept: Vec<Mutex<Vec<Option<ID3D12Resource>>>>,
    /// Where the search for free dynamic slots starts, and which are taken
    /// (by lists not yet reset).
    next: Mutex<(u32, Vec<bool>)>,
    /// How many dynamic slots are free (changed under `next`'s lock): with
    /// fewer than a table needs, the search is not made. Once the world
    /// had taken them all, every later table of the frame searched every
    /// slot before it was refused.
    free: std::sync::atomic::AtomicU32,
    /// The shortest table that found no run of free slots since slots were
    /// last given back (changed under `next`'s lock; `u32::MAX`: none).
    /// Until something is given back the free runs only shrink, so a table
    /// of that length or longer is refused without the search. Enough
    /// slots were free, but in pieces, and every such table walked the
    /// whole ring before it was refused: 0.33 ms each with 131,072 slots,
    /// 1.0 s of the recording threads in a frame that refused 3,143 (run
    /// 45), under the lock all of them need. (A free run that lies across
    /// the place the search starts from is seen by neither.)
    too_long: std::sync::atomic::AtomicU32,
    /// View heaps: the bytes per element of the buffer view at each title
    /// slot, for shaders that pick a buffer by an index they compute
    /// (`victory_shader::emit::SLOT_STRIDES`): an upload buffer of
    /// `SLOT_STRIDES_COUNT` dwords, its GPU address and its mapping.
    strides: Option<(ID3D12Resource, u64, usize)>,
    /// View heaps, with the structure builder asked for: the title address
    /// of the buffer or ray-tracing structure view at each title slot, for
    /// shaders that take it out of a descriptor they pick by an index they
    /// compute (`victory_shader::emit::SLOT_VIEW_ADDRESSES`): an upload
    /// buffer of two dwords a slot, its GPU address and its mapping. After
    /// victory-fork pull request 994.
    addresses: Option<(ID3D12Resource, u64, usize)>,
    /// View heaps: the mark table of translated shaders' write marks
    /// (`victory_shader::emit`, "Write marks"; `crate::marks`): per slot,
    /// two dwords, the first bit and page offset of the unordered buffer
    /// view there (`VIEW_MARK_NONE`: its writes are not marked). Written
    /// whenever the slot is ([`HostHeap::keep`]). An upload buffer of the
    /// heap's slots, its GPU address, its mapping and its slots.
    marks: Option<(ID3D12Resource, u64, usize, u32)>,
    pub(crate) buffers: BufferViews<std::sync::Arc<crate::shadow::Shadow>>,
    /// Title slots holding a view of a texture the title may fill itself (a
    /// placed texture, a texture object), with the frame from which they
    /// are due: such a texture is uploaded when a draw reads it through a
    /// table; one a shader picks by a computed index is brought up to date
    /// from here ([`HostHeap::texture_slots_due`]).
    textures: Mutex<TextureQueue>,
    /// Per title slot, the frame (plus one; 0: never) in which a draw last
    /// read it through a table, or made a table it lies in (with
    /// [`TABLED_READ`] set if the draw read it). The draw that
    /// reads such a slot brings its texture up to date itself and the slot
    /// then leaves `textures`; a write of a slot that was a table's within
    /// the last [`TABLED_FRAMES`] frames is due only [`TABLED_GRACE`]
    /// frames later, by when its draw has read it. A title that builds a
    /// table per draw writes thousands of texture views a frame into slots
    /// only tables read (the title: some 2,500 a frame in its
    /// opening cutscene, a few hundred textures over and over), and every
    /// one of them stood in the queue of textures picked by index, ahead
    /// of or behind the ones shaders do pick by index.
    tabled: Vec<std::sync::atomic::AtomicU32>,
    /// The title memory each placed texture view of this heap describes,
    /// by slot and by address ([`Spans`]): which slots a newer view of
    /// other memory there makes stale, and which to look at again when the
    /// runtime maps, unmaps or writes memory ([`remapped`]).
    placed: std::sync::Arc<Mutex<Spans>>,
    /// Material records the card read for this heap
    /// (`crate::material_demand`).
    pub(crate) material_demand: crate::material_demand::Mailbox,
    /// Made only while material records are read: per title slot, the
    /// number of the title's last write of it, counted by `material_clock`.
    /// A slot may be written again, and leave the queue, before the card
    /// hands back the index a record held.
    material_epochs: std::sync::OnceLock<Box<[std::sync::atomic::AtomicU64]>>,
    material_clock: std::sync::atomic::AtomicU64,
}

/// The spans of every view heap made ([`HostHeap::placed`]), for the
/// texture code, which has no heap at hand ([`next_start`],
/// [`first_viewed`]).
static PLACED_SPANS: Mutex<Vec<std::sync::Weak<Mutex<Spans>>>> = Mutex::new(Vec::new());

/// Where the next texture placed inside `[start, end)` starts, past the
/// first bytes of the one at `start` ([`HEAD`]): what is there of the
/// texture at `start` ends there, whatever its full size. `None`: no view
/// in a slot shaders pick by index describes memory in that range.
pub(crate) fn next_start(start: u64, end: u64) -> Option<u64> {
    let heaps = PLACED_SPANS.lock().unwrap_or_else(|e| e.into_inner());
    heaps
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .filter_map(|spans| spans.lock().unwrap_or_else(|e| e.into_inner()).next_start(start, end))
        .min()
}

/// The lowest level that a view of the placed texture at `start` shows,
/// over the slots shaders pick by index that hold one now. The title
/// views a texture from the first level it holds. `None`: no such slot.
pub(crate) fn first_viewed(start: u64) -> Option<u32> {
    let heaps = PLACED_SPANS.lock().unwrap_or_else(|e| e.into_inner());
    heaps
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .filter_map(|spans| spans.lock().unwrap_or_else(|e| e.into_inner()).first_viewed(start))
        .min()
}

/// Levels on demand (`crate::tiles`): where the placed textures start
/// whose first level with memory changed since a texture pass last took
/// them ([`HostHeap::texture_views_remade`]).
static RESHOWN: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// The first level with memory of the placed texture at `start` changed:
/// its slots are made again at the next texture pass.
pub(crate) fn levels_changed(start: u64) {
    RESHOWN.lock().unwrap_or_else(|e| e.into_inner()).push(start);
}

/// Levels on demand (`crate::tiles`): where the placed textures start
/// whose last upload left a held level without tiles (the card had no
/// room, or no heap could be had), and when (`crate::host::moment`). A
/// texture pass makes their slots due again once the card has room
/// ([`HostHeap::texture_slots_waiting`]).
static WAITING: Mutex<std::collections::BTreeMap<u64, u64>> = Mutex::new(std::collections::BTreeMap::new());

/// The last upload of the placed texture at `start` left a held level
/// without tiles. A texture picked by index is uploaded once and then
/// only when its memory is found changed, and its staging buffer goes
/// with the upload: without this the level stayed empty (the texture
/// blurred, or black if no level had tiles) for as long as the title left
/// its memory alone, where a texture refused for want of room is tried
/// again.
pub(crate) fn levels_waiting(start: u64) {
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).insert(start, crate::host::moment());
}

/// Title memory by the placed texture views that describe it: per span of
/// memory (its start, its end) the resource description of the texture
/// there and the title slots holding a view of it. A span is the memory of
/// the mips a view shows ([`crate::resource::placed_bytes_from`]). A console
/// title need not clear a descriptor when it streams a texture out, so a
/// newer view that cannot be there together with an older one is all that
/// says the older one is stale ([`Rule`] says when that is). Views of one
/// span with different descriptions (the formats of one family, say) are one
/// texture's, and so are views of one description at one start that show
/// more and fewer of its mips. (Descriptions are compared without the filler
/// words the title leaves in them, [`crate::resource::placed_description`].)
///
/// Spans may overlap: a texture's smallest levels lie at the start of its
/// memory, and a title that holds only those puts the next texture where
/// the larger ones would be (the title: three 512x256 textures of 0x2c000
/// bytes each whose starts are 0x2000 and 0x4000 apart, all three read by
/// the same frame's draws). What is there of a texture ends where the next
/// one starts.
/// No host heap is needed, so it is tested without one.
#[derive(Default)]
pub(crate) struct Spans {
    /// When a newer view makes an older one stale.
    rule: Rule,
    spans: std::collections::BTreeMap<(u64, u64), ([u32; 14], std::collections::BTreeSet<u32>)>,
    by_slot: std::collections::HashMap<u32, (u64, u64)>,
    /// The level each slot's view starts at ([`Spans::shows`]).
    first: std::collections::HashMap<u32, u32>,
    /// The bytes a placed texture takes, by resource description and first
    /// mip: worked out once for each (a title writes the views of a few
    /// thousand descriptions, thousands of times a frame).
    sizes: std::collections::HashMap<([u32; 14], u32), Option<u64>>,
    /// With [`displaced_recovery`]: slots a newer view made stale while
    /// they showed a texture, with the record they showed (only the key:
    /// no host resource is kept for them), and where the next look at
    /// them starts ([`HostHeap::recover_displaced`]).
    displaced: std::collections::BTreeMap<u32, u64>,
    displaced_next: u32,
}

/// When a view the title writes makes an older view of other memory stale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Rule {
    /// When it starts where the older one starts: at the same address
    /// with another length, or within the older one's first [`HEAD`]
    /// bytes (or the older within its own). No two textures share the
    /// block of their smallest levels. A view that only reaches into the
    /// address range of an older texture's larger levels says nothing.
    #[default]
    Start,
    /// Whenever their spans at full size overlap (the rule until run
    /// 188, where it made null the slots of textures packed as above:
    /// each view written took its neighbours out of the queue and off the
    /// card, for good). `VICTORY_STALE_TEXTURES=overlap`.
    Overlap,
    /// Never; a view that would be stale under [`Rule::Overlap`] is left
    /// out of the spans instead. `VICTORY_STALE_TEXTURES=0`, as before.
    Never,
}

/// The first bytes of a placed texture that are its own under
/// [`Rule::Start`]: the 4 KiB block its smallest levels share. (Of 129
/// pairs of overlapping neighbours in the listings of runs 188 and 189,
/// none starts closer than 0x2000 to the one before it.)
const HEAD: u64 = 4096;

impl Spans {
    /// Up to `max` slots of [`Spans::displaced`], going on from where the
    /// last batch ended.
    fn displaced_batch(&mut self, max: usize) -> Vec<(u32, u64)> {
        let batch: Vec<_> = self
            .displaced
            .range(self.displaced_next..)
            .chain(self.displaced.range(..self.displaced_next))
            .take(max)
            .map(|(&slot, &key)| (slot, key))
            .collect();
        if let Some(&(slot, _)) = batch.last() {
            self.displaced_next = slot.wrapping_add(1);
        }
        batch
    }

    /// The bytes of title memory the mips from `first` on of a placed
    /// texture with the resource description `desc` take.
    fn bytes(&mut self, desc: &[u32; 14], first: u32) -> Option<u64> {
        *self
            .sizes
            .entry((crate::resource::placed_description(desc), first))
            .or_insert_with(|| crate::resource::placed_bytes_from(desc, first))
    }

    /// The spans that overlap `[start, end)`, each taken to end where the
    /// next one starts: of the start before, those that reach past `start`
    /// at the size their view shows, and every span that starts inside.
    fn overlapping(&self, start: u64, end: u64) -> Vec<(u64, u64)> {
        // The spans of one start are one texture's, and no other texture's
        // reaches over it: only those of the start before may reach here.
        let before = self.spans.range(..(start, 0)).next_back().map(|(&(at, _), _)| at);
        let before = before.into_iter().flat_map(|at| self.spans.range((at, 0)..=(at, u64::MAX)));
        before
            .map(|(&span, _)| span)
            .filter(|span| span.1 > start)
            .chain(self.spans.range((start, 0)..(end, 0)).map(|(&span, _)| span))
            .collect()
    }

    /// The spans that say otherwise than that a texture with the resource
    /// description `desc` lies in `[start, end)`: by [`Rule`], those that
    /// start within the first [`HEAD`] bytes of it (or it of them), or those
    /// that overlap it. A span of the same start and the same description is
    /// the same texture's, whatever of its mips the other view shows.
    fn against(&self, start: u64, end: u64, desc: &[u32; 14]) -> Vec<(u64, u64)> {
        let desc = crate::resource::placed_description(desc);
        let mut spans: Vec<(u64, u64)> = match self.rule {
            Rule::Start => self
                .spans
                .range((start.saturating_sub(HEAD - 1), 0)..(start.saturating_add(HEAD), 0))
                .map(|(&span, _)| span)
                .collect(),
            Rule::Overlap | Rule::Never => self.overlapping(start, end),
        };
        spans.retain(|span| {
            let same = span.0 == start && (span.1 == end || self.spans.get(span).is_some_and(|s| s.0 == desc));
            !same
        });
        spans
    }

    /// Slot `index` holds a view of a texture with the resource description
    /// `desc` in `[start, end)`. `newer`: the title wrote it now, so the
    /// slots whose views cannot be there with it ([`Rule`]) are stale: they
    /// are returned, and forgotten. Otherwise (a draw made the slot from
    /// what it held), and under [`Rule::Never`], a view that clashes with
    /// other spans is left out.
    fn note(&mut self, index: u32, start: u64, end: u64, desc: &[u32; 14], newer: bool) -> Vec<u32> {
        if self.by_slot.get(&index) == Some(&(start, end)) {
            return Vec::new();
        }
        self.forget(index);
        if end <= start {
            return Vec::new();
        }
        let mut stale = Vec::new();
        for span in self.against(start, end, desc) {
            if !newer || self.rule == Rule::Never {
                return stale;
            }
            if let Some((_, slots)) = self.spans.remove(&span) {
                // The first of them are said with both spans, to tell a
                // replacement from a neighbour in one launch's log.
                static SAID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if SAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 48 {
                    tracing::info!(
                        slot = index,
                        start = format_args!("{start:#x}"),
                        end = format_args!("{end:#x}"),
                        stale_start = format_args!("{:#x}", span.0),
                        stale_end = format_args!("{:#x}", span.1),
                        stale_slots = slots.len(),
                        rule = ?self.rule,
                        "a placed texture view makes older views of other memory stale"
                    );
                }
                for slot in slots {
                    self.by_slot.remove(&slot);
                    stale.push(slot);
                }
            }
        }
        let desc = crate::resource::placed_description(desc);
        self.spans.entry((start, end)).or_insert_with(|| (desc, Default::default())).1.insert(index);
        self.by_slot.insert(index, (start, end));
        stale
    }

    /// The view in slot `index` starts at level `first` of its texture.
    fn shows(&mut self, index: u32, first: u32) {
        self.first.insert(index, first);
    }

    /// The spans that start at `start`, whatever their end.
    fn at_start(&self, start: u64) -> impl Iterator<Item = (&(u64, u64), &([u32; 14], std::collections::BTreeSet<u32>))> {
        self.spans.range((start, 0)..=(start, u64::MAX))
    }

    /// The lowest level a view of the texture at `start` starts at.
    fn first_viewed(&self, start: u64) -> Option<u32> {
        self.at_start(start)
            .flat_map(|(_, span)| span.1.iter())
            .filter_map(|slot| self.first.get(slot))
            .min()
            .copied()
    }

    /// The first span that starts inside `[start, end)` past the first
    /// [`HEAD`] bytes of it.
    fn next_start(&self, start: u64, end: u64) -> Option<u64> {
        let from = start.saturating_add(HEAD);
        if from >= end {
            return None;
        }
        self.spans.range((from, 0)..(end, 0)).next().map(|(&(at, _), _)| at)
    }

    /// The slots that hold a view of the texture at `start`.
    fn slots_at(&self, start: u64) -> Vec<u32> {
        self.at_start(start).flat_map(|(_, span)| span.1.iter().copied()).collect()
    }

    /// Slot `index` holds no placed texture view any more.
    fn forget(&mut self, index: u32) {
        self.first.remove(&index);
        let Some(at) = self.by_slot.remove(&index) else {
            return;
        };
        if let Some(span) = self.spans.get_mut(&at) {
            span.1.remove(&index);
            if span.1.is_empty() {
                self.spans.remove(&at);
            }
        }
    }

    /// What the spans say of a view of a texture with the resource
    /// description `desc` in `[start, end)` in slot `index`, and the first
    /// span that speaks against it.
    fn claim(&self, index: u32, start: u64, end: u64, desc: &[u32; 14]) -> (Claim, Option<(u64, u64)>) {
        if self.by_slot.get(&index) == Some(&(start, end)) {
            return (Claim::Own, None);
        }
        let spans = self.against(start, end, desc);
        match spans.first() {
            None => (Claim::Free, None),
            Some(&span) if spans.iter().all(|s| s.0 == start) => (Claim::SameStart, Some(span)),
            Some(&span) => (Claim::Other, Some(span)),
        }
    }

    /// The slots whose texture's memory overlaps `[start, end)`.
    fn slots_in(&self, start: u64, end: u64) -> Vec<u32> {
        self.overlapping(start, end)
            .iter()
            .filter_map(|at| self.spans.get(at))
            .flat_map(|span| span.1.iter().copied())
            .collect()
    }

    /// The slots of the textures that hold bytes of `[start, end)`, for a
    /// write there: as [`Spans::slots_in`], but when a texture starts at
    /// `start` itself the one before it is left out (it ends where that
    /// one starts, and a request that fills a texture begins at its first
    /// byte: `slots_in` would name its neighbour with every such write).
    fn slots_holding(&self, start: u64, end: u64) -> Vec<u32> {
        if end <= start {
            return Vec::new();
        }
        let mut slots: Vec<u32> = Vec::new();
        if self.at_start(start).next().is_none() {
            // Of the start before, the spans that reach past `start`.
            if let Some(at) = self.spans.range(..(start, 0)).next_back().map(|(&(at, _), _)| at) {
                for (&(_, span_end), span) in self.at_start(at) {
                    if span_end > start {
                        slots.extend(span.1.iter().copied());
                    }
                }
            }
        }
        slots.extend(self.spans.range((start, 0)..(end, 0)).flat_map(|(_, span)| span.1.iter().copied()));
        slots
    }
}

/// Title memory written behind the guards' back whose bytes are there
/// (DirectStorage's reads into physical pages, at the addresses the title
/// has them mapped; `crate::watch::title_memory_written`), since a texture
/// pass last took the ranges ([`page_written`]). Kept apart from
/// [`REMAPPED`] so that what such writes do is counted by itself.
static PAGE_WRITTEN: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());

/// Since the start: such ranges kept for a pass; ranges dropped because no
/// pass took them; slots of placed textures that held bytes of them when a
/// pass took them; of those, the slots whose texture had been uploaded
/// before the write (out of the queue, or in it after an upload): they are
/// the ones that kept the bytes from before, and the ones made due when
/// `VICTORY_PAGE_WRITES` asks for it.
pub(crate) static PAGE_WRITES: [std::sync::atomic::AtomicU64; 4] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 4];

/// Frames from the pass that takes a written range to the look at its
/// slots: the levels of one texture arrive in a few requests, and a look
/// two frames on takes those of the next frame with it.
const PAGE_WRITE_AFTER: u64 = 2;

/// The bytes of a write the guards do not see are in title memory `[at,
/// at + len)`: the placed textures there changed with no descriptor
/// written and nothing mapped. Kept for the next texture pass
/// ([`HostHeap::texture_slots_page_written`]), as [`remapped`] keeps its
/// ranges; locks one mutex and calls nothing.
pub(crate) fn page_written(at: usize, len: usize) {
    use std::sync::atomic::Ordering::Relaxed;
    /// Ranges kept until a pass takes them.
    const MOST: usize = 1 << 15;
    let (start, end) = (at as u64, at.saturating_add(len) as u64);
    if end <= SPANNED[0].load(Relaxed) || start >= SPANNED[1].load(Relaxed) {
        return;
    }
    let mut ranges = PAGE_WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    PAGE_WRITES[0].fetch_add(1, Relaxed);
    if let Some(last) = ranges.last_mut()
        && last.1 == start
    {
        last.1 = end;
        return;
    }
    if ranges.len() >= MOST {
        PAGE_WRITES[1].fetch_add(ranges.len() as u64, Relaxed);
        ranges.clear();
    }
    ranges.push((start, end));
}

/// Whose the memory of a texture view is, by [`Spans`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Claim {
    /// The slot's own.
    Own,
    /// No other texture's.
    Free,
    /// Another texture's that starts at the same address.
    SameStart,
    /// Another texture's.
    Other,
}

/// What the last whole look at the heap's placed texture views found
/// ([`HostHeap::texture_sweep`]): views; of them shown by their host slot;
/// in the queue; read through tables lately; without memory; then the ones
/// with memory that nothing brings up to date, by [`Claim`] (own, free,
/// same start, other), and of all those the ones whose host slot shows
/// another view. And since the start: the ones put back into the queue.
pub(crate) static SWEPT: [std::sync::atomic::AtomicU64; 11] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 11];

/// A look at the heap under way ([`HostHeap::texture_sweep`]).
struct TextureSweep {
    /// The next slot.
    next: u32,
    /// What it counted.
    counts: [u64; 10],
    /// When (milliseconds) it went on last.
    last_ms: u64,
    /// With [`sweep_resume`]: per part, the slot after the last one it put
    /// back, where the next look at that part starts.
    resume: Vec<u32>,
}

static SWEEP: Mutex<TextureSweep> =
    Mutex::new(TextureSweep { next: 0, counts: [0; 10], last_ms: 0, resume: Vec::new() });

/// `VICTORY_SWEEP_RESUME=1` (off by default): the look at a part of the
/// heap goes on after the last slot it put back, not from the part's first
/// slot (after DexNom/VIctory b8ff22e). It puts back at most 128 slots a
/// part; with streaming making the same low slots stale at every visit,
/// the ones further on in a part of 32,768 were never reached.
fn sweep_resume() -> bool {
    #[cfg(test)]
    if SWITCHED_ON.get() {
        return true;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SWEEP_RESUME").is_some_and(|v| v == "1"))
}

/// `VICTORY_DISPLACED_RECOVER=1` (off by default): a slot whose texture a
/// newer view made stale is remembered, and put back into the queue once
/// no view claims its memory any more while it still holds the same record
/// and its memory is there (after DexNom/VIctory b8ff22e). Without it, such
/// a slot waits for the sweep to come round, whose put-backs are also
/// shared with slots never made. A slot cleared by the title also gives
/// up the memory it claimed, made or not.
pub(crate) fn displaced_recovery() -> bool {
    #[cfg(test)]
    if SWITCHED_ON.get() {
        return true;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_DISPLACED_RECOVER").is_some_and(|v| v == "1"))
}

#[cfg(test)]
thread_local! {
    /// Tests: [`sweep_resume`] and [`displaced_recovery`] on, for the test's
    /// own thread (the others go on with the switches as they are).
    static SWITCHED_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Title memory the runtime mapped, unmapped or is about to write since a
/// texture pass last took the ranges ([`remapped`]).
static REMAPPED: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());

/// The lowest start and the highest end of any span of [`Spans`] so far
/// (the start is `u64::MAX` until there is one): what the runtime does to
/// memory outside is none of the texture pass's business.
static SPANNED: [std::sync::atomic::AtomicU64; 2] = [
    std::sync::atomic::AtomicU64::new(u64::MAX),
    std::sync::atomic::AtomicU64::new(0),
];

/// Since the start: slots whose view a newer view of other memory there
/// made stale; ranges of memory noted as mapped, unmapped or written;
/// ranges dropped because no pass took them; slots made due by such
/// ranges; and made slots whose memory was found unmapped (made null).
pub(crate) static STALE: [std::sync::atomic::AtomicU64; 5] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 5];

/// The slots of view heaps that bakes named lately (`crate::draw`'s bake
/// draws, the texture views their data picks by index), by heap and slot,
/// with the moment they were named ([`crate::host::now_ms`]).
static BAKE_NAMED: Mutex<Option<std::collections::HashMap<(usize, u32), u64>>> = Mutex::new(None);

/// Until when [`BAKE_NAMED`] is looked at by slot writes
/// ([`crate::host::now_ms`] + 1; 0: never named).
static BAKE_NAMED_UNTIL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How long a slot a bake named is watched.
const BAKE_NAMED_MS: u64 = 10_000;

/// Since the last frame-rate line (`crate::host`'s bake line): slots a bake
/// named whose host descriptor the host changed (another resource, or
/// none) while a submission was in the air, and of them those made null;
/// then, with [`stale_after_flights`], slots a newer view made stale while
/// submissions were in the air, left as they were, and of those the ones
/// made null since, once those submissions had finished.
static BAKE_SLOTS: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

/// [`BAKE_SLOTS`], taken for the line.
pub(crate) fn bake_slot_changes() -> [u64; 4] {
    BAKE_SLOTS.each_ref().map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed))
}

/// A bake draw names `slots` of `heap` (texture views its data picks by
/// index): watched for a while ([`BAKE_NAMED_MS`]).
pub(crate) fn bake_named(heap: &HostHeap, slots: &[u32]) {
    if slots.is_empty() {
        return;
    }
    let now = crate::host::now_ms();
    let key = std::ptr::from_ref(heap) as usize;
    let mut named = BAKE_NAMED.lock().unwrap_or_else(|e| e.into_inner());
    let named = named.get_or_insert_with(Default::default);
    named.retain(|_, at| *at + BAKE_NAMED_MS > now);
    for &slot in slots {
        named.insert((key, slot), now);
    }
    BAKE_NAMED_UNTIL.store(now + BAKE_NAMED_MS + 1, std::sync::atomic::Ordering::Relaxed);
}

/// Whether slot writes look at [`BAKE_NAMED`] now.
fn bake_watch_on() -> bool {
    let until = BAKE_NAMED_UNTIL.load(std::sync::atomic::Ordering::Relaxed);
    until != 0 && crate::host::now_ms() < until
}

/// The host changed what slot `index` of `heap` shows ([`HostHeap::keep`]):
/// counted if a bake named it lately and a submission is in the air, which
/// may be that bake, not yet run by the card. A descriptor in a
/// shader-visible heap is read by the card when it runs the draw, not when
/// the draw is recorded: such a bake reads what the slot shows now.
fn bake_slot_changed(heap: &HostHeap, index: u32, nulled: bool) {
    if !crate::host::in_air() {
        return;
    }
    let key = (std::ptr::from_ref(heap) as usize, index);
    let named = BAKE_NAMED.lock().unwrap_or_else(|e| e.into_inner());
    if named.as_ref().is_some_and(|named| named.contains_key(&key)) {
        BAKE_SLOTS[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if nulled {
            BAKE_SLOTS[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Slots a newer view of other memory makes stale ([`HostHeap::note_buffer`])
/// are made null only once the submissions in the air when that was found
/// have finished (`VICTORY_STALE_AFTER_FLIGHTS=1`; off by default: at once,
/// as before).
///
/// Why: on the console a view the title writes changes its own slot and
/// no other; the host makes the other slots that describe the same memory
/// null at once, and a slot of a shader-visible heap is read by the card
/// when it runs a draw. A draw recorded and sent before the newer view was
/// written, and not yet run, then reads nothing where its texture was: a
/// texture streaming in gets a newer view (more mips at the same address)
/// while frames that sampled the older one are still on the card. Without
/// flights sent ahead that is at most the one submission the card runs;
/// with the relay it is every submission in the air, hundreds of lists. A
/// frame drawn so is drawn again; a bake, which runs once, keeps what it
/// read.
fn stale_after_flights() -> bool {
    #[cfg(test)]
    if STALE_TEST.get().is_some() {
        return true;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_STALE_AFTER_FLIGHTS").is_some_and(|v| v == "1"))
}

#[cfg(test)]
thread_local! {
    /// Tests: [`stale_after_flights`] on for the test's own thread, with
    /// the submissions started and finished given here in place of the
    /// host's.
    static STALE_TEST: std::cell::Cell<Option<(u64, u64)>> = const { std::cell::Cell::new(None) };
}

/// The submissions started and finished (`crate::host::started`,
/// `crate::host::finished`), for [`stale_after_flights`].
fn stale_clock() -> (u64, u64) {
    #[cfg(test)]
    if let Some(clock) = STALE_TEST.get() {
        return clock;
    }
    (crate::host::started(), crate::host::finished())
}

/// Stale slots left showing their view until submissions finish
/// ([`stale_after_flights`]): heap, slot, the key the slot was made from
/// (made null later only if it still holds it), and the submissions
/// started when it was found stale.
static STALE_LATER: Mutex<Vec<(usize, u32, u64, u64)>> = Mutex::new(Vec::new());

/// Whether [`STALE_LATER`] holds anything (looked at without its lock).
static STALE_LATER_ANY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Of `later` (heap, slot, key, started), the entries of heap `heap` whose
/// submissions have finished by `finished`: taken out and returned as
/// (slot, key); the rest stay.
fn stale_due(later: &mut Vec<(usize, u32, u64, u64)>, heap: usize, finished: u64) -> Vec<(u32, u64)> {
    let mut due = Vec::new();
    later.retain(|&(h, slot, key, started)| {
        if h == heap && started <= finished {
            due.push((slot, key));
            false
        } else {
            true
        }
    });
    due
}

/// Since the start: slots a pass found unmapped whose host slot it left as
/// it was, because another write had replaced the view it looked at
/// ([`HostHeap::texture_slot_gone`]): each was a good view that would have
/// been made null.
pub(crate) static GONE_REFUSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// When a view the title writes makes older views of other memory stale
/// ([`Rule`]): where it starts in the block of their smallest levels, by
/// default; `VICTORY_STALE_TEXTURES=0`: never, as before;
/// `VICTORY_STALE_TEXTURES=overlap`: wherever their spans overlap, the rule
/// until run 188.
fn stale_rule() -> Rule {
    static RULE: std::sync::OnceLock<Rule> = std::sync::OnceLock::new();
    *RULE.get_or_init(|| match std::env::var("VICTORY_STALE_TEXTURES").as_deref() {
        Ok("0") => Rule::Never,
        Ok("overlap") => Rule::Overlap,
        _ => Rule::Start,
    })
}

/// The runtime mapped or unmapped title memory `[at, at + len)`, or is
/// about to write it through the kernel (`crate::watch`): the placed
/// textures there may have arrived, gone, or changed with no descriptor
/// written. The range is kept for the next texture pass
/// ([`HostHeap::texture_slots_remapped`]). Called with the runtime's own
/// locks held: this locks one mutex of its own and calls nothing.
pub(crate) fn remapped(at: usize, len: usize) {
    use std::sync::atomic::Ordering::Relaxed;
    /// Ranges kept until a pass takes them.
    /// (A world's first minutes note tens of thousands between two passes:
    /// 131,072 were dropped in one run at the 32,768 there were, and 163,840
    /// in our last, while the title loaded and before the first pass.)
    const MOST: usize = 1 << 18;
    let (start, end) = (at as u64, at.saturating_add(len) as u64);
    if end <= SPANNED[0].load(Relaxed) || start >= SPANNED[1].load(Relaxed) {
        return;
    }
    let mut ranges = REMAPPED.lock().unwrap_or_else(|e| e.into_inner());
    STALE[1].fetch_add(1, Relaxed);
    // Pages are mapped one after the other.
    if let Some(last) = ranges.last_mut()
        && last.1 == start
    {
        last.1 = end;
        return;
    }
    if ranges.len() >= MOST {
        // No pass runs (no shader that picks textures by index is bound):
        // the slots there are looked at when they are due, as before.
        STALE[2].fetch_add(ranges.len() as u64, Relaxed);
        ranges.clear();
    }
    ranges.push((start, end));
}

/// Frames for which a slot a draw read through a table counts as a table's
/// slot ([`HostHeap::tabled`]).
const TABLED_FRAMES: u64 = 240;

/// Frames a write of a table's slot waits in the queue for the draw that
/// reads it, before the slot is taken for one shaders pick by index.
/// Frames as `crate::frame::current` counts them: in its opening cutscene
/// The title begins two or three of them per frame it presents
/// (435 begun in the 353 s of run 52's, for some 140 presented).
const TABLED_GRACE: u64 = 8;

/// In [`HostHeap::tabled`]: a draw read the slot in that frame.
const TABLED_READ: u32 = 1 << 31;

/// Since the start: writes of texture views into slots that count as a
/// table's (due later), and slots that left the queue because a draw read
/// them through a table.
pub(crate) static TABLED_WRITES: [std::sync::atomic::AtomicU64; 2] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 2];

/// Whether slots draws read through tables leave the queue of textures
/// picked by index; `VICTORY_TABLED_TEXTURES=1` keeps them in it as
/// before.
fn tabled_out() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TABLED_TEXTURES").is_none_or(|v| v != "1"))
}

/// What the queue of textures picked by index knows of a title slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Waiting {
    /// The frame from which the slot is due.
    due: u64,
    /// The frame it was written in.
    written: u64,
    /// Which write of the queue put it here: the pass hands it back with
    /// what it did, so a slot written again meanwhile stays as written.
    write: u64,
    /// Looks in a row that found nothing to upload (its memory not mapped,
    /// or no more of it than at the last upload).
    looks: u8,
    /// Bytes of its memory that were mapped when it was last uploaded
    /// without all of it there; 0: not uploaded yet.
    mapped: u64,
    /// Looks at the slot's memory that found it changed since the slot was
    /// written: uploads it needed again ([`relook_wait`]).
    changes: u8,
    /// A slot uploaded whole waits for looks at its memory, not for an
    /// upload: `due` is the frame of the next look
    /// ([`TextureQueue::relooks_due`]), and the pass never sees it.
    relook: Option<Relook>,
    /// A look found the memory changed: the slot waits for its upload
    /// again, behind every slot never uploaded ([`TextureQueue::due`]).
    stale: bool,
    /// Put into the queue because a material record the card read names
    /// it (`crate::material_demand`), not by a write of the title: its
    /// memory is looked at again before the upload
    /// ([`HostHeap::material_upload_allowed`]).
    recovered: bool,
}

impl Waiting {
    /// A slot written (or whose memory changed) in `frame`, due from `due`.
    fn new(due: u64, written: u64, write: u64) -> Waiting {
        Waiting { due, written, write, looks: 0, mapped: 0, changes: 0, relook: None, stale: false, recovered: false }
    }
}

/// What a look at a slot uploaded whole compares with: where its memory is
/// (address, bytes), its fingerprint at the upload (never 0), the looks in
/// a row that found it as it was, and the frame of the upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Relook {
    pub(crate) at: (u64, u64),
    pub(crate) print: u64,
    pub(crate) looks: u8,
    pub(crate) uploaded: u64,
}

/// Frames from a whole upload of a texture picked by index to the first
/// look at its memory, and from each look to the next that finds it as it
/// was: the title may fill a texture's memory after it wrote the view, with
/// no descriptor write and nothing from the runtime to tell of it.
const RELOOK_FRAMES: [u64; 5] = [4, 12, 40, 120, 400];

/// Frames until look number `looks` (0: the first after an upload) at a
/// texture the title's memory of which a look found changed `changes`
/// times since the slot was written: the waits of [`RELOOK_FRAMES`], each
/// doubled by every change, at most the longest of them. A texture the
/// title keeps writing is so uploaded again every few hundred frames, not
/// every few. `None` after the last look of a texture that holds still.
pub(crate) fn relook_wait(looks: u8, changes: u8) -> Option<u64> {
    let base = *RELOOK_FRAMES.get(usize::from(looks))?;
    let longest = RELOOK_FRAMES[RELOOK_FRAMES.len() - 1];
    Some((base << changes.min(7)).min(longest))
}

/// One frame's looks at the textures uploaded whole whose next look is due
/// at `frame` (the ones that have waited longest first): at most `most`
/// looks and `span` of time, the rest wait for the next frame. `print`
/// is the fingerprint of the memory `(address, bytes)` now; it runs
/// without the queue's lock, which the title's descriptor writes take.
/// `changed` is told of each slot whose memory differs from the upload's.
/// Returns the looks made and those that found a change.
fn relook(
    queue: &Mutex<TextureQueue>,
    frame: u64,
    most: usize,
    span: std::time::Duration,
    print: impl Fn(u64, u64) -> u64,
    mut changed: impl FnMut(u32, &Relook),
) -> (u64, u64) {
    let due = queue.lock().unwrap_or_else(|e| e.into_inner()).relooks_due(frame, most);
    let began = std::time::Instant::now();
    let (mut looks, mut found) = (0, 0);
    for (index, write, look) in due {
        if began.elapsed() > span {
            break;
        }
        let differs = print(look.at.0, look.at.1) != look.print;
        looks += 1;
        if differs {
            found += 1;
            changed(index, &look);
        }
        queue.lock().unwrap_or_else(|e| e.into_inner()).relooked(index, write, frame, differs);
    }
    (looks, found)
}

/// The queue of title slots whose texture view waits to be brought up to
/// date for shaders that pick textures by index: which are due, and in
/// which order (no host heap needed, so it is tested without one).
#[derive(Default)]
pub(crate) struct TextureQueue {
    slots: std::collections::BTreeMap<u32, Waiting>,
    writes: u64,
    /// Slots held back because the card had no room for the host copy
    /// they need ([`TextureQueue::hold`]), each with the write it was
    /// held as.
    held: std::collections::BTreeMap<u32, u64>,
}

impl TextureQueue {
    /// A texture view was written into slot `index` in `frame`, due from
    /// frame `due`. `seen`: it was not written, a draw or the pass made
    /// the host slot from a view the queue may hold already, which then
    /// stays as it is.
    fn written(&mut self, index: u32, frame: u64, due: u64, seen: bool) {
        if seen && self.slots.contains_key(&index) {
            return;
        }
        self.writes += 1;
        let write = self.writes;
        self.slots.insert(index, Waiting::new(due, frame, write));
    }

    fn remove(&mut self, index: u32) {
        self.slots.remove(&index);
    }

    /// Up to `max` slots due at `frame`, each with its write number, in
    /// the order to serve them:
    /// - first the ones looked at before (their memory was not mapped, or
    ///   only part of it): the look is a query, and memory that has
    ///   arrived since is what the title streamed in last;
    /// - then the others, the newest write first and the oldest next, in
    ///   turns: slot order served the low slots of a backlog of
    ///   25,000-40,000 while the textures of the shot on screen waited
    ///   (run 52: 98% of the drawn pixels had no colour texture in the
    ///   first 3D frames);
    /// - last the textures a look found changed after their upload
    ///   ([`TextureQueue::relooked`]), the one that has waited longest
    ///   first: every slot never uploaded goes ahead of them, a slot
    ///   uploaded whole is not in this list at all until a look finds it
    ///   changed (the looks themselves are [`TextureQueue::relooks_due`]).
    ///
    /// Only what the pass can take is sorted (PR #1031 of victory-fork): of
    /// a queue of 25,000-40,000 slots the pass takes at most 4,096, so each
    /// class is first cut to the entries that can make the list (a
    /// selection, which keeps the order the full sorts gave: write numbers
    /// are unique).
    fn due(&self, frame: u64, max: usize) -> Vec<(u32, u64)> {
        if max == 0 {
            return Vec::new();
        }
        // (slot, write number) for the first two classes; (frame found
        // changed, slot, write number) for the last.
        let mut again: Vec<(u32, u64)> = Vec::new();
        let mut fresh: Vec<(u32, u64)> = Vec::new();
        let mut stale: Vec<(u64, u32, u64)> = Vec::new();
        for (&index, w) in self.slots.iter().filter(|(_, w)| w.due <= frame && w.relook.is_none()) {
            if w.stale {
                stale.push((w.due, index, w.write));
                continue;
            }
            let class = if w.looks > 0 || w.mapped > 0 { &mut again } else { &mut fresh };
            class.push((index, w.write));
        }
        // By the number of the write, counted over the run.
        let newest_first = |a: &(u32, u64), b: &(u32, u64)| b.1.cmp(&a.1);
        if again.len() > max {
            again.select_nth_unstable_by(max, newest_first);
            again.truncate(max);
        }
        again.sort_unstable_by(newest_first);
        let mut order = again;
        // The fresh ones: the newest and the oldest in turns, the newest
        // first, as many as there is room for.
        let take = (max - order.len()).min(fresh.len());
        if take > 0 {
            let (new_count, old_count) = (take.div_ceil(2), take / 2);
            if new_count < fresh.len() {
                fresh.select_nth_unstable_by(new_count, newest_first);
            }
            let (newest, rest) = fresh.split_at_mut(new_count);
            newest.sort_unstable_by(newest_first);
            let oldest_at = rest.len() - old_count;
            if old_count > 0 && oldest_at > 0 {
                rest.select_nth_unstable_by(oldest_at, newest_first);
            }
            let oldest = &mut rest[oldest_at..];
            oldest.sort_unstable_by(|a, b| newest_first(b, a));
            order.reserve(take);
            for (position, &entry) in newest.iter().enumerate() {
                order.push(entry);
                order.extend(oldest.get(position).copied());
            }
        }
        // The first found changed first (the frame of the look, then the
        // slot's write).
        let room = max - order.len();
        if room > 0 && !stale.is_empty() {
            let key = |&(found, _, write): &(u64, u32, u64)| (found, write);
            if stale.len() > room {
                stale.select_nth_unstable_by_key(room, key);
                stale.truncate(room);
            }
            stale.sort_unstable_by_key(key);
            order.extend(stale.iter().map(|&(_, index, write)| (index, write)));
        }
        order
    }

    /// The entry of `index` if it is still the write the pass was handed.
    fn same(&mut self, index: u32, write: u64) -> Option<&mut Waiting> {
        self.slots.get_mut(&index).filter(|w| w.write == write)
    }

    /// Slot `index` is up to date: out of the queue until written again.
    fn done(&mut self, index: u32, write: u64) {
        if self.same(index, write).is_some() {
            self.slots.remove(&index);
        }
    }

    /// Nothing to upload at slot `index` now: looked at again after 1, 2,
    /// 4, ... frames, at most `most`.
    fn later(&mut self, index: u32, write: u64, frame: u64, most: u64) {
        if let Some(w) = self.same(index, write) {
            w.due = frame + (1u64 << w.looks.min(16)).min(most.max(1));
            w.looks = w.looks.saturating_add(1);
            // What a look at its memory compared with is out of date.
            (w.relook, w.stale) = (None, false);
        }
    }

    /// The card has no room for the host copy slot `index` needs: it is
    /// not handed out again before frame `due`, or before the card has
    /// room again ([`TextureQueue::room`]). Nothing else of it changes: it
    /// is not a slot "waiting for its memory", and when it is due it has
    /// its old place among the slots never looked at. (It stayed due, so
    /// every pass on a full card looked at the same few hundred slots
    /// again and stopped there, and the slots behind them were never
    /// reached: the ones whose host copy exists and only wants bringing up
    /// to date, and the ones whose memory went, which is what lets host
    /// copies go and makes room.)
    fn hold(&mut self, index: u32, write: u64, due: u64) {
        if let Some(w) = self.same(index, write) {
            w.due = w.due.max(due);
            self.held.insert(index, write);
        }
    }

    /// The card has room again at `frame`: the slots held back for lack of
    /// it are due now, as if they had never been held. Returns how many.
    fn room(&mut self, frame: u64) -> usize {
        let held = std::mem::take(&mut self.held);
        let mut back = 0;
        for (index, write) in held {
            if let Some(w) = self.same(index, write) {
                w.due = w.due.min(frame);
                back += 1;
            }
        }
        back
    }

    /// Slot `index` was uploaded with `mapped` bytes of its memory there,
    /// not all of it: looked at again from frame `due`.
    fn partly(&mut self, index: u32, write: u64, due: u64, mapped: u64) {
        if let Some(w) = self.same(index, write) {
            (w.due, w.looks, w.mapped) = (due, 0, mapped.max(1));
            (w.relook, w.stale) = (None, false);
        }
    }

    /// Slot `index` was uploaded whole in `frame` from memory `at`
    /// (address, bytes), which had fingerprint `print`: the pass is done
    /// with it, and looks at its memory follow ([`relook_wait`]) for a
    /// title that fills a texture's memory after it wrote the view. True
    /// if this upload was one a look had asked for.
    fn uploaded_whole(&mut self, index: u32, write: u64, frame: u64, at: (u64, u64), print: u64) -> bool {
        let Some(w) = self.same(index, write) else {
            return false;
        };
        let again = w.stale;
        match relook_wait(0, w.changes) {
            Some(wait) => {
                (w.due, w.looks, w.mapped, w.stale) = (frame + wait, 0, 0, false);
                w.relook = Some(Relook { at, print, looks: 0, uploaded: frame });
            }
            None => {
                self.slots.remove(&index);
            }
        }
        again
    }

    /// The slots uploaded whole whose next look is due at `frame`, at most
    /// `max`, the one that has waited longest first.
    fn relooks_due(&self, frame: u64, max: usize) -> Vec<(u32, u64, Relook)> {
        let mut due: Vec<(u64, u32, u64, Relook)> = self
            .slots
            .iter()
            .filter_map(|(&index, w)| w.relook.filter(|_| w.due <= frame).map(|r| (w.due, index, w.write, r)))
            .collect();
        if due.len() > max {
            due.select_nth_unstable_by_key(max, |&(due, index, ..)| (due, index));
            due.truncate(max);
        }
        due.sort_unstable_by_key(|&(due, index, ..)| (due, index));
        due.into_iter().map(|(_, index, write, r)| (index, write, r)).collect()
    }

    /// A look at slot `index`'s memory in `frame` found it `changed` since
    /// the upload or as it was. As it was: the next look, or none after the
    /// last. Changed: the slot waits for an upload from this frame on,
    /// behind the slots never uploaded ([`TextureQueue::due`]), and the
    /// looks after that one are further apart.
    fn relooked(&mut self, index: u32, write: u64, frame: u64, changed: bool) {
        let Some(w) = self.same(index, write) else {
            return;
        };
        let Some(r) = w.relook else {
            return;
        };
        if changed {
            w.changes = w.changes.saturating_add(1);
            (w.due, w.looks, w.mapped, w.stale) = (frame, 0, 0, true);
            w.relook = None;
            return;
        }
        match relook_wait(r.looks.saturating_add(1), w.changes) {
            Some(wait) => {
                w.due = frame + wait;
                w.relook = Some(Relook { looks: r.looks + 1, ..r });
            }
            None => {
                self.slots.remove(&index);
            }
        }
    }

    /// The memory slot `index`'s texture lies in was mapped, unmapped or
    /// written in `frame`: the slot is looked at from frame `due` as one
    /// written now, whatever was known of it (a pass at work on it does
    /// not take it out).
    fn changed(&mut self, index: u32, frame: u64, due: u64) {
        self.writes += 1;
        let write = self.writes;
        let recovered = self.slots.get(&index).is_some_and(|w| w.recovered);
        let due = self.slots.get(&index).map_or(due, |w| w.due.min(due));
        self.slots.insert(index, Waiting { recovered, ..Waiting::new(due, frame, write) });
    }

    /// Slots that wait for an upload (not those uploaded whole that only
    /// wait for a look at their memory).
    fn uploads_waiting(&self) -> usize {
        self.slots.values().filter(|w| w.relook.is_none()).count()
    }

    /// Bytes of slot `index`'s memory that were mapped at its last upload
    /// (0: not uploaded, or all of it was).
    fn mapped(&self, index: u32) -> u64 {
        self.slots.get(&index).map_or(0, |w| w.mapped)
    }

    /// For the frame line: slots in the queue; of them due for an upload
    /// at `frame`; waiting for their memory (looked at, never uploaded);
    /// uploaded in part; the frames the oldest due one has waited since its
    /// write; and slots never uploaded; slots uploaded whole that wait for
    /// a look at their memory; slots a look found changed, waiting for
    /// their upload; and looks due.
    fn census(&self, frame: u64) -> [u64; 9] {
        let mut c = [self.slots.len() as u64, 0, 0, 0, 0, 0, 0, 0, 0];
        for w in self.slots.values() {
            if w.relook.is_some() {
                c[6] += 1;
                c[8] += u64::from(w.due <= frame);
                continue;
            }
            if w.mapped > 0 {
                c[3] += 1;
            } else if w.looks > 0 {
                c[2] += 1;
            }
            if w.stale {
                c[7] += 1;
            } else if w.mapped == 0 {
                c[5] += 1;
            }
            if w.due <= frame {
                c[1] += 1;
                c[4] = c[4].max(frame.saturating_sub(w.written));
            }
        }
        c
    }
}

/// Whether `view` is of a texture the title may fill itself: a texture it
/// placed in memory, or a texture object.
pub(crate) fn is_texture_view(view: &View) -> bool {
    match view {
        View::Placed(false, resource, _) => resource[0] != 1,
        View::Shader(r, _) => crate::resource::buffer_bytes(*r as crate::object::This).is_none(),
        _ => false,
    }
}

/// Buffer views left null because the host would refuse them
/// ([`buffer_view_ok`]), since the start.
pub(crate) static REFUSED_VIEWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What became of buffer views that did not fit the host copy of their
/// memory as they were, since the start (see [`view_fates`] for the order).
static VIEW_FATES: [std::sync::atomic::AtomicU64; 10] = [const { std::sync::atomic::AtomicU64::new(0) }; 10];
const FATE_NO_ELEMENTS: usize = 0;
const FATE_RAW_OFF_16: usize = 1;
const FATE_OWN_TYPED: usize = 2;
const FATE_OWN_RAW: usize = 3;
const FATE_OWN_STRUCTURED: usize = 4;
const FATE_UNORDERED_OFF_ELEMENT: usize = 5;
const FATE_UNORDERED_STRUCTURED: usize = 6;
const FATE_OFF_ELEMENT: usize = 7;
const FATE_CONSTANT_OFF_256: usize = 8;
const FATE_CONSTANT_OWN: usize = 9;

fn fate(which: usize) {
    VIEW_FATES[which].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// [`REFUSED_VIEWS`] and [`VIEW_FATES`] for the frame statistics.
pub(crate) fn view_fates() -> String {
    let f = |i: usize| VIEW_FATES[i].load(std::sync::atomic::Ordering::Relaxed);
    format!(
        "{} (of no elements {}, raw ones not on 16 bytes {}); buffer views left null off their copy's \
         elements: {} read, {} unordered; unordered structured views not on 16 bytes, left structured: {}; \
         buffer views on a copy of their own range: {} typed, {} raw, {} structured; constant buffer          views left null as not on 256 bytes of the host's copy: {}, made on a copy of their own range: {}{}",
        REFUSED_VIEWS.load(std::sync::atomic::Ordering::Relaxed),
        f(FATE_NO_ELEMENTS),
        f(FATE_RAW_OFF_16),
        f(FATE_OFF_ELEMENT),
        f(FATE_UNORDERED_OFF_ELEMENT),
        f(FATE_UNORDERED_STRUCTURED),
        f(FATE_OWN_TYPED),
        f(FATE_OWN_RAW),
        f(FATE_OWN_STRUCTURED),
        f(FATE_CONSTANT_OFF_256),
        f(FATE_CONSTANT_OWN),
        crate::shadow::stale_report(),
    )
}

/// Whether the host device is gone, and why (the first time it is asked
/// after that): for passes that make many views, to name the one that did
/// it.
pub(crate) fn device_removed() -> Option<String> {
    let host = host()?;
    // SAFETY: a plain query.
    let reason = unsafe { host.device.GetDeviceRemovedReason() };
    reason.err().map(|e| e.to_string())
}

/// Writes of title view-heap slots with a placed texture view and with a
/// view of a texture object, since the start (diagnostic counts).
pub(crate) static PLACED_TEXTURE_SLOTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) static OBJECT_TEXTURE_SLOTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bytes per element of a buffer view, as the console's descriptor holds
/// them for indexed reads (structured: the stride; raw: 1, the index is the
/// byte; typed: the format's); `None` if `view` is not a buffer view.
fn view_stride(view: &View) -> Option<u32> {
    const BUFFER: u32 = 1;
    let (words, unordered): (&[u32; 10], bool) = match view {
        View::Shader(_, Some(d)) if d[1] == BUFFER => (d, false),
        View::Unordered(_, _, Some(d)) if d[1] == BUFFER => (d, true),
        View::Placed(unordered, resource, d) if resource[0] == BUFFER => {
            (d[..10].try_into().ok()?, *unordered)
        }
        _ => return None,
    };
    // SAFETY: buffer views' union members.
    let (stride, raw, format) = unsafe {
        if unordered {
            let d = uav_desc(words);
            let b = d.Anonymous.Buffer;
            (b.StructureByteStride, b.Flags.0 & D3D12_BUFFER_UAV_FLAG_RAW.0 != 0, d.Format)
        } else {
            let d = srv_desc(words);
            let b = d.Anonymous.Buffer;
            (b.StructureByteStride, b.Flags.0 & D3D12_BUFFER_SRV_FLAG_RAW.0 != 0, d.Format)
        }
    };
    Some(if stride > 0 {
        stride
    } else if raw {
        1
    } else {
        crate::resource::element(format.0 as u32).map_or(0, |(bytes, _)| bytes as u32)
    })
}

/// Host heaps' GPU ranges (start, bytes, bytes per slot, sampler), for
/// device-removed reports: a shader indexing a heap past its end faults.
static GPU_RANGES: Mutex<Vec<(u64, u64, u64, bool)>> = Mutex::new(Vec::new());

/// For device-removed reports: where host GPU address `va` lies against
/// the host descriptor heaps.
pub(crate) fn near_heaps(va: u64) -> String {
    let ranges = GPU_RANGES.lock().unwrap_or_else(|e| e.into_inner());
    let mut text = String::new();
    for &(start, bytes, step, sampler) in ranges.iter() {
        let kind = if sampler { "sampler" } else { "view" };
        let place = if va < start {
            format!("{:#x} below", start - va)
        } else if va < start + bytes {
            format!("inside, slot {}", (va - start) / step)
        } else {
            format!("{:#x} past its end (slot {})", va - start - bytes, (va - start) / step)
        };
        text.push_str(&format!("; {kind} heap {start:#x}+{bytes:#x}: {place}"));
    }
    text
}

/// The host's limit for shader-visible view heaps on resource binding
/// tiers 1 and 2; tier 3 hardware may take more.
const PORTABLE_VIEWS: u32 = 1_000_000;

/// The size of every host view heap on a host that takes more than
/// [`PORTABLE_VIEWS`] (AMD): the title's heap (a little over 1,000,000
/// views in the retail game) plus [`DYNAMIC`] fits with room to spare, and
/// the last slot is the null slot every view index at or past it reads.
const UNCAPPED_VIEWS: u32 = 1 << 21;

/// Picks, once, the null slot at the end of every host view heap and tells
/// the translator now (`set_view_heap_limit`) instead of when the title's
/// heap is made: a shader keeps the limit it was translated with, and the
/// title makes 25 pipelines (their shaders translated at once) before it
/// makes its view heap. Called before a shader is translated and before a
/// view heap is made.
///
/// Every host clamps. A host that caps view heaps at [`PORTABLE_VIEWS`]
/// (NVIDIA) hung on a view index past the heap; one that does not (AMD
/// RDNA3, RX 7900 XT) took a page fault and removed the device on entering
/// the world: a compute program read view slot 16,777,088 (0xffff80) of a
/// heap of 1,507,328, past its end. Unclamped, that index is undefined on
/// any host.
pub(crate) fn probe_view_limit() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Some(host) = host() else { return };
        let fits = |count: u32| {
            // SAFETY: plain heap creation; the heap is released at once.
            let made: windows::core::Result<ID3D12DescriptorHeap> = unsafe {
                host.device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        NumDescriptors: count,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    })
            };
            made.is_ok()
        };
        let views = if fits(UNCAPPED_VIEWS) {
            UNCAPPED_VIEWS
        } else {
            PORTABLE_VIEWS
        };
        tracing::info!(
            views,
            "host view heaps hold {views} views; view indices are clamped to the null slot at the end from the first shader on"
        );
        victory_shader::emit::set_view_heap_limit(views - 1);
    });
}

/// Slots past the title's in a view heap, for tables a draw needs made
/// afresh (constant buffers copied per draw). A list holds the slots it
/// took until it is reset (its submissions have run) or released.
/// 2^16 ran out in the world ("no free dynamic descriptor slots", run 11;
/// upstream PR #336 takes 2^18). On a host heap capped at 1,000,000 views
/// (NVIDIA) these come out of the title's slots, whose highest seen is
/// about 720,000: 2^17 left 868,000 of them, and ran out in the opening
/// cutscene (a frame asks for up to 16,000 tables while the frame before
/// still holds its own; a draw refused its table read slots never made
/// for it, and the card stopped within frames). 2^18 leaves 737,000; a
/// table past them is made afresh in these slots like any other.
pub(crate) const DYNAMIC: u32 = 1 << 18;

impl Drop for HostHeap {
    /// What [`stale_after_flights`] and the bake watch keep of this heap
    /// goes with it: another heap made at the same place is not this one.
    fn drop(&mut self) {
        let key = std::ptr::from_ref(self) as usize;
        if STALE_LATER_ANY.load(std::sync::atomic::Ordering::Relaxed) {
            STALE_LATER.lock().unwrap_or_else(|e| e.into_inner()).retain(|e| e.0 != key);
        }
        if let Some(named) = BAKE_NAMED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            named.retain(|&(heap, _), _| heap != key);
        }
    }
}

impl HostHeap {
    pub(crate) fn new(sampler: bool, count: u32) -> Option<Self> {
        let host = host()?;
        let kind = if sampler {
            D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
        } else {
            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
        };
        let make = |count: u32| -> windows::core::Result<ID3D12DescriptorHeap> {
            // SAFETY: plain heap creation.
            unsafe {
                host.device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: kind,
                        NumDescriptors: count,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    })
            }
        };
        let extra = if sampler { 0 } else { DYNAMIC };
        // A view heap ends at the null slot shaders clamp view indices to
        // (`probe_view_limit`), whatever the title's own count: the title's
        // slots, then the dynamic ones, then null up to and with that slot.
        if !sampler {
            probe_view_limit();
        }
        let null = if sampler {
            0
        } else {
            victory_shader::emit::view_heap_limit()
        };
        let (heap, count, size) = if null == 0 {
            match make(count + extra) {
                Ok(heap) => (heap, count, count + extra),
                Err(e) => {
                    tracing::warn!(error = %e, count, sampler, "no host descriptor heap");
                    return None;
                }
            }
        } else {
            let size = null + 1;
            let fitted = if count + extra <= null {
                count
            } else {
                // shortcut: descriptors past the host's limit are left out; a title that reaches
                // them needs remapping. They read the null slot instead.
                tracing::warn!(count, "host view heap capped at {size}; views past it read a null slot");
                null - extra
            };
            match make(size) {
                Ok(heap) => (heap, fitted, size),
                Err(e) => {
                    tracing::warn!(error = %e, count, size, "no host view heap");
                    return None;
                }
            }
        };
        // SAFETY: a live heap.
        let (cpu, step) = unsafe {
            (
                heap.GetCPUDescriptorHandleForHeapStart().ptr,
                host.device.GetDescriptorHandleIncrementSize(kind),
            )
        };
        tracing::debug!(count, sampler, "host descriptor heap");
        // SAFETY: a live shader-visible heap.
        let gpu = unsafe { heap.GetGPUDescriptorHandleForHeapStart().ptr };
        GPU_RANGES.lock().unwrap_or_else(|e| e.into_inner()).push((
            gpu,
            u64::from(size) * u64::from(step),
            u64::from(step),
            sampler,
        ));
        let strides = (!sampler)
            .then(|| {
                let bytes = u64::from(victory_shader::emit::SLOT_STRIDES_COUNT) * 4;
                let (buffer, data) = host.upload_buffer(bytes)?;
                // SAFETY: the buffer's whole mapping; a live buffer.
                let gpu = unsafe {
                    std::ptr::write_bytes(data, 0, bytes as usize);
                    buffer.GetGPUVirtualAddress()
                };
                crate::host::name(&buffer, || "element strides by view heap slot".to_owned());
                Some((buffer, gpu, data as usize))
            })
            .flatten();
        // (16 MiB of upload memory a view heap: only for the builder's runs.)
        let addresses = (!sampler && crate::pipeline::builder_allowed())
            .then(|| {
                let bytes = u64::from(victory_shader::emit::SLOT_STRIDES_COUNT) * 8;
                let (buffer, data) = host.upload_buffer(bytes)?;
                // SAFETY: the buffer's whole mapping; a live buffer.
                let gpu = unsafe {
                    std::ptr::write_bytes(data, 0, bytes as usize);
                    buffer.GetGPUVirtualAddress()
                };
                crate::host::name(&buffer, || "view addresses by view heap slot".to_owned());
                Some((buffer, gpu, data as usize))
            })
            .flatten();
        let marks = (!sampler)
            .then(|| {
                let bytes = u64::from(size) * 8;
                let (buffer, data) = host.upload_buffer(bytes)?;
                // SAFETY: the buffer's whole mapping (every slot unmarked
                // until a view is made there); a live buffer.
                let gpu = unsafe {
                    std::ptr::write_bytes(data, 0xFF, bytes as usize);
                    buffer.GetGPUVirtualAddress()
                };
                crate::host::name(&buffer, || "write marks by view heap slot".to_owned());
                Some((buffer, gpu, data as usize, size))
            })
            .flatten();
        let h = HostHeap {
            heap,
            cpu,
            gpu,
            step,
            count,
            dynamic: extra,
            sampler,
            made: Mutex::new(vec![0; count as usize]),
            made_fast: (0..count).map(|_| std::sync::atomic::AtomicU64::new(0)).collect(),
            shown: Mutex::new(Vec::new()),
            copied: Mutex::new(Vec::new()),
            kept: (0..(count + extra).div_ceil(KEPT_PIECE))
                .map(|_| Mutex::new(vec![None; KEPT_PIECE as usize]))
                .collect(),
            next: Mutex::new((0, vec![false; DYNAMIC as usize])),
            free: std::sync::atomic::AtomicU32::new(DYNAMIC),
            too_long: std::sync::atomic::AtomicU32::new(u32::MAX),
            strides,
            addresses,
            marks,
            buffers: BufferViews::new(),
            textures: Mutex::default(),
            material_demand: Default::default(),
            material_epochs: Default::default(),
            material_clock: Default::default(),
            tabled: (0..if sampler { 0 } else { count })
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect(),
            placed: {
                let spans = std::sync::Arc::<Mutex<Spans>>::default();
                if !sampler {
                    PLACED_SPANS
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(std::sync::Arc::downgrade(&spans));
                }
                spans
            },
        };
        // A slot never written is undefined: shaders reading one (a slot
        // the title wrote in a way we do not know, or a table entry no
        // used-resource entry names) read a wild address. Every slot starts
        // null instead.
        let started = std::time::Instant::now();
        if let Err(e) = h.fill_null(kind, size) {
            tracing::warn!(error = %e, "host descriptor heap not made null");
        }
        tracing::debug!(ms = started.elapsed().as_millis(), "host descriptor heap made null");
        Some(h)
    }

    /// Makes the first `total` slots null: one null slot in a CPU-side heap,
    /// doubled by copies to a block, the block copied over the heap.
    fn fill_null(&self, kind: D3D12_DESCRIPTOR_HEAP_TYPE, total: u32) -> windows::core::Result<()> {
        const BLOCK: u32 = 1 << 16;
        let Some(host) = host() else { return Ok(()) };
        // SAFETY: plain heap creation and copies inside live heaps.
        unsafe {
            let block: ID3D12DescriptorHeap =
                host.device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: kind,
                    NumDescriptors: BLOCK,
                    ..Default::default()
                })?;
            let start = block.GetCPUDescriptorHandleForHeapStart();
            let at = |i: u32| D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: start.ptr + (i * self.step) as usize,
            };
            if self.sampler {
                null_sampler(start);
            } else {
                null_view(start);
            }
            let mut filled = 1;
            while filled < BLOCK {
                let n = filled.min(BLOCK - filled);
                host.device.CopyDescriptorsSimple(n, at(filled), start, kind);
                filled += n;
            }
            let mut done = 0;
            while done < total {
                let n = BLOCK.min(total - done);
                host.device.CopyDescriptorsSimple(n, self.cpu(done), start, kind);
                done += n;
            }
        }
        Ok(())
    }

    /// A plain point sampler at slot `index`, for sampler slots without a
    /// view.
    fn null_sampler(&self, index: u32) {
        null_sampler(self.cpu(index));
    }
}

/// A plain point sampler at `at`.
fn null_sampler(at: D3D12_CPU_DESCRIPTOR_HANDLE) {
    let Some(host) = host() else { return };
    let desc = D3D12_SAMPLER_DESC {
        Filter: D3D12_FILTER_MIN_MAG_MIP_POINT,
        AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
        ..Default::default()
    };
    // SAFETY: a slot of a sampler heap of ours.
    unsafe { host.device.CreateSampler(&desc, at) };
}

impl HostHeap {
    /// `n` consecutive free dynamic slots (view heaps only), taken until
    /// [`HostHeap::release`]: the first's index.
    pub(crate) fn dynamic(&self, n: u32) -> Option<u32> {
        if self.sampler || n == 0 || n > self.dynamic {
            return None;
        }
        #[cfg(feature = "diagnostics")]
        let started = std::time::Instant::now();
        #[cfg(feature = "diagnostics")]
        crate::copystat::add(crate::copystat::C::DynamicAsked, 1);
        let mut guard = self.next.lock().unwrap_or_else(|e| e.into_inner());
        let (next, taken) = &mut *guard;
        let enough = self.free.load(std::sync::atomic::Ordering::Relaxed) >= n
            && n < self.too_long.load(std::sync::atomic::Ordering::Relaxed);
        let n = n as usize;
        // Search from `next` on, then from the start.
        let starts = (*next as usize..self.dynamic as usize).chain(0..*next as usize);
        let mut run = 0;
        let mut found = None;
        let mut last = usize::MAX;
        for at in starts.take(if enough { DYNAMIC as usize } else { 0 }) {
            // A run cannot wrap past the end.
            if at != last.wrapping_add(1) {
                run = 0;
            }
            last = at;
            run = if taken[at] { 0 } else { run + 1 };
            if run == n {
                found = Some(at + 1 - n);
                break;
            }
        }
        let Some(first) = found else {
            if enough {
                self.too_long.fetch_min(n as u32, std::sync::atomic::Ordering::Relaxed);
            }
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| tracing::warn!(n, "no free dynamic descriptor slots: a table is not made"));
            #[cfg(feature = "diagnostics")]
            {
                use crate::copystat::{C, add};
                add(C::DynamicRefused, 1);
                add(C::DynamicNs, started.elapsed().as_nanos() as u64);
            }
            return None;
        };
        #[cfg(feature = "diagnostics")]
        crate::copystat::add(crate::copystat::C::DynamicNs, started.elapsed().as_nanos() as u64);
        taken[first..first + n].fill(true);
        self.free.fetch_sub(n as u32, std::sync::atomic::Ordering::Relaxed);
        *next = ((first + n) % DYNAMIC as usize) as u32;
        Some(self.count + first as u32)
    }

    /// The title slots this heap has host slots for; fewer than the title's
    /// when the host caps its size.
    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    /// The GPU address of the table of element strides by title slot (view
    /// heaps).
    /// The table of view addresses by title slot (view heaps, with the
    /// builder asked for): its GPU address and its buffer, for a list to
    /// hold.
    pub(crate) fn addresses(&self) -> Option<(u64, &ID3D12Resource)> {
        self.addresses.as_ref().map(|a| (a.1, &a.0))
    }

    pub(crate) fn strides_gpu(&self) -> Option<u64> {
        self.strides.as_ref().map(|s| s.1)
    }

    /// The table's buffer, for a list to hold.
    pub(crate) fn strides_buffer(&self) -> Option<&ID3D12Resource> {
        self.strides.as_ref().map(|s| &s.0)
    }

    /// The mark table (view heaps), for the root SRV of shaders that mark
    /// their writes: its buffer and GPU address.
    pub(crate) fn marks(&self) -> Option<(&ID3D12Resource, u64)> {
        self.marks.as_ref().map(|m| (&m.0, m.1))
    }

    /// Slot `index`'s mark table entry: the view [`make`] just made there,
    /// if it is an unordered view of `resource` whose writes are marked;
    /// none otherwise.
    fn set_mark(&self, index: u32, resource: Option<&ID3D12Resource>) {
        let Some((_, _, data, slots)) = &self.marks else {
            MARK.with(|m| m.set(None));
            return;
        };
        let made = MARK.with(std::cell::Cell::take);
        let entry = match (made, resource) {
            (Some((made, entry)), Some(r)) if made == windows::core::Interface::as_raw(r) as usize => entry,
            _ => (victory_shader::emit::VIEW_MARK_NONE, 0),
        };
        if index >= *slots {
            return;
        }
        // SAFETY: two dwords of the mapped table, which has two per slot.
        unsafe {
            let at = (*data as *mut u32).add(2 * index as usize);
            at.write_volatile(entry.0);
            at.add(1).write_volatile(entry.1);
        }
    }

    /// Up to `max` title slots whose texture view is due at `frame`, each
    /// with its write number, in the order to serve them
    /// ([`TextureQueue::due`]), and the queue's counts
    /// ([`TextureQueue::census`]).
    pub(crate) fn texture_slots_due(&self, frame: u64, max: usize) -> (Vec<(u32, u64)>, [u64; 9]) {
        let textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        (if max == 0 { Vec::new() } else { textures.due(frame, max) }, textures.census(frame))
    }

    /// Of the title slots `among`, in their order, those the queue holds
    /// due for an upload at `frame`, with the write each was handed out as
    /// (as [`HostHeap::texture_slots_due`]): a bake's own sources, which go
    /// first in the pass before it.
    pub(crate) fn texture_slots_due_of(&self, frame: u64, among: &[u32]) -> Vec<(u32, u64)> {
        let textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<(u32, u64)> = Vec::new();
        for &index in among {
            if out.iter().any(|&(i, _)| i == index) {
                continue;
            }
            if let Some(w) = textures.slots.get(&index).filter(|w| w.due <= frame && w.relook.is_none()) {
                out.push((index, w.write));
            }
        }
        out
    }

    /// Every title slot of the queue that is due at `frame`, and whether
    /// it was uploaded before (in part, or whole and waiting for another
    /// look): for the card accounting line
    /// (`crate::resource::card_accounting`).
    pub(crate) fn texture_slots_all_due(&self, frame: u64) -> Vec<(u32, bool)> {
        let textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        textures
            .slots
            .iter()
            .filter(|(_, w)| w.due <= frame)
            .map(|(&index, w)| (index, w.mapped > 0 || w.relook.is_some()))
            .collect()
    }

    /// The texture view of title slot `index` (as handed out with `write`)
    /// was brought up to date: due again from frame `again` (uploaded with
    /// that many bytes of its memory there, not all), or not until the
    /// slot is written again.
    pub(crate) fn texture_slot_done(&self, index: u32, write: u64, again: Option<(u64, u64)>) {
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        match again {
            Some((frame, mapped)) => textures.partly(index, write, frame, mapped),
            None => textures.done(index, write),
        }
    }

    /// Title slot `index` was uploaded whole in `frame` from memory `at`
    /// (address, bytes) with fingerprint `print`: see
    /// [`TextureQueue::uploaded_whole`]. True if a look had asked for the
    /// upload.
    pub(crate) fn texture_slot_uploaded_whole(&self, index: u32, write: u64, frame: u64, at: (u64, u64), print: u64) -> bool {
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        textures.uploaded_whole(index, write, frame, at, print)
    }

    /// The looks at textures uploaded whole that are due at `frame`
    /// ([`relook`]): the slots a look finds changed wait for their upload.
    /// Returns the looks made and those that found a change.
    pub(crate) fn texture_slots_relook(
        &self,
        frame: u64,
        most: usize,
        span: std::time::Duration,
        print: impl Fn(u64, u64) -> u64,
        changed: impl FnMut(u32, &Relook),
    ) -> (u64, u64) {
        relook(&self.textures, frame, most, span, print, changed)
    }

    /// Nothing to upload at title slot `index` now (its memory is not
    /// mapped, or no more of it than before): looked at again after 1, 2,
    /// 4, ... frames, at most `most`.
    pub(crate) fn texture_slot_later(&self, index: u32, write: u64, frame: u64, most: u64) {
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        textures.later(index, write, frame, most);
    }

    /// See [`TextureQueue::hold`].
    pub(crate) fn texture_slot_held(&self, index: u32, write: u64, due: u64) {
        self.textures.lock().unwrap_or_else(|e| e.into_inner()).hold(index, write, due);
    }

    /// See [`TextureQueue::room`].
    pub(crate) fn texture_slots_room(&self, frame: u64) -> usize {
        self.textures.lock().unwrap_or_else(|e| e.into_inner()).room(frame)
    }

    /// Bytes of title slot `index`'s memory that were mapped when it was
    /// last uploaded in part (0: not uploaded, or whole).
    pub(crate) fn texture_slot_mapped(&self, index: u32) -> u64 {
        self.textures.lock().unwrap_or_else(|e| e.into_inner()).mapped(index)
    }

    /// Takes the ranges of title memory the runtime mapped, unmapped or
    /// wrote since the last call ([`remapped`]) and makes the slots whose
    /// placed texture lies there due from the next frame (by when a write
    /// under way has landed): memory that arrived is uploaded, memory that
    /// went makes the slot null ([`HostHeap::texture_slot_gone`]), and a
    /// texture streamed into the place of another of the same description
    /// is uploaded although no descriptor says so. Slots tables read are
    /// left to their draws. Returns how many slots it made due.
    /// shortcut: the ranges are kept in one place for all heaps; the heap
    /// whose pass runs first takes them (a title has one view heap).
    pub(crate) fn texture_slots_remapped(&self, frame: u64) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        self.texture_slots_page_written(frame);
        let ranges = std::mem::take(&mut *REMAPPED.lock().unwrap_or_else(|e| e.into_inner()));
        if ranges.is_empty() {
            return 0;
        }
        let mut slots: Vec<u32> = {
            let placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
            ranges.iter().flat_map(|&(start, end)| placed.slots_in(start, end)).collect()
        };
        slots.sort_unstable();
        slots.dedup();
        // A table's slot waits for its draw, as a write of it does: the
        // draw takes it out again. Left out altogether, a slot that shaders
        // also pick by index kept what it showed before.
        let lately: Vec<bool> = slots.iter().map(|&slot| self.tabled_lately(slot, frame)).collect();
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        for (&slot, lately) in slots.iter().zip(lately) {
            textures.changed(slot, frame, frame + if lately { TABLED_GRACE } else { 1 });
        }
        STALE[3].fetch_add(slots.len() as u64, Relaxed);
        slots.len()
    }

    /// Goes on with a look at all of the heap's placed texture views, a
    /// part at a time: a view whose memory is there, whose host slot shows
    /// nothing (or another view) and which is not in the queue will stay
    /// that way until the title writes it again, and a shader that picks it
    /// reads black. One whose memory no newer view describes otherwise goes
    /// back into the queue; the others are counted ([`SWEPT`]).
    /// `VICTORY_TEXTURE_SWEEP=0` turns it off, `count` only counts.
    pub(crate) fn texture_sweep(&self, heap: &super::Heap, frame: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        /// Slots looked at at a time, and the least time between.
        const SLOTS: u32 = 1 << 15;
        const EVERY_MS: u64 = 100;
        static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
        let mode = *MODE.get_or_init(|| match std::env::var("VICTORY_TEXTURE_VIEW_SWEEP").as_deref() {
            Ok("0") => 0,
            Ok("count") => 1,
            _ => 2,
        });
        if mode == 0 || self.sampler {
            return;
        }
        let now = crate::host::now_ms();
        let Ok(mut sweep) = SWEEP.try_lock() else {
            return;
        };
        if now.saturating_sub(sweep.last_ms) < EVERY_MS {
            return;
        }
        sweep.last_ms = now;
        if mode == 2 && displaced_recovery() {
            self.recover_displaced(|index| heap.handle(index), frame);
        }
        let (from, mut counts) = (sweep.next.min(self.count), sweep.counts);
        let to = from.saturating_add(SLOTS).min(self.count);
        let part = (from / SLOTS) as usize;
        let resume = if sweep_resume() {
            sweep.resume.get(part).copied().filter(|r| (from..to).contains(r)).unwrap_or(from)
        } else {
            from
        };
        let next = self.sweep_part(from..to, resume, |index| heap.handle(index), frame, mode == 2, &mut counts);
        if sweep_resume() {
            if sweep.resume.len() <= part {
                sweep.resume.resize(part + 1, 0);
            }
            sweep.resume[part] = next;
        }
        sweep.next = to;
        sweep.counts = counts;
        if to >= self.count {
            for (to, counted) in SWEPT.iter().zip(counts) {
                to.store(counted, Relaxed);
            }
            (sweep.next, sweep.counts) = (0, [0; 10]);
        }
    }

    /// With [`displaced_recovery`]: a look at up to 32 slots a newer view
    /// made stale ([`Spans::displaced`]). One that still holds the record
    /// it showed, is not made, is not a table's lately, whose memory is
    /// (partly) there and which no other view claims goes back into the
    /// queue; one that holds another record, or was made since, is let go.
    /// Returns how many went back.
    fn recover_displaced(&self, handle: impl Fn(u32) -> Option<u64>, frame: u64) -> u64 {
        let batch = self.placed.lock().unwrap_or_else(|e| e.into_inner()).displaced_batch(32);
        let mut restored = 0;
        for (index, key) in batch {
            // Its memory asked outside the locks; its claim is asked again
            // below, under them.
            let extent = self.mapped_displaced_view(index, key);
            let handle = handle(index).filter(|&at| crate::texture::committed(at as *const u8, 12));
            let made = self.made.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: a descriptor slot of the title's heap, there.
            let words = handle.map(|at| unsafe { (at as *const [u32; 3]).read_unaligned() });
            let current = made.get(index as usize).copied() == Some(0)
                && words == Some([VIEW_TAG, key as u32, (key >> 32) as u32]);
            {
                let mut placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
                if placed.displaced.get(&index) != Some(&key) {
                    continue;
                }
                if !current {
                    placed.displaced.remove(&index);
                    continue;
                }
                let Some((resource, start, end)) = extent else {
                    continue;
                };
                if self.tabled_lately(index, frame)
                    || !matches!(placed.claim(index, start, end, &resource).0, Claim::Free | Claim::Own)
                {
                    continue;
                }
                // The span is the slot's again, under `made` as a title
                // write takes it; a newer view before the upload makes the
                // slot stale once more. The key stays until the slot is
                // made or written again.
                placed.note(index, start, end, &resource, false);
            }
            let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
            if textures.slots.contains_key(&index) {
                continue;
            }
            textures.written(index, frame, frame + 1, false);
            restored += 1;
            drop(textures);
            drop(made);
        }
        if restored != 0 {
            static TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let before = TOTAL.fetch_add(restored, std::sync::atomic::Ordering::Relaxed);
            let total = before + restored;
            if before == 0 || total / 256 != before / 256 {
                tracing::info!(
                    frame,
                    restored,
                    total,
                    "texture views a newer view had made stale go back into the queue: no view claims their memory any more"
                );
            }
        }
        restored
    }

    /// [`HostHeap::recover_displaced`]: the resource description and span
    /// of the placed texture view `key` names, if no other view claims it
    /// and some of its memory is mapped (asked outside the slot's lock).
    fn mapped_displaced_view(&self, index: u32, key: u64) -> Option<([u32; 14], u64, u64)> {
        let View::Placed(false, resource, desc) = view_at(key as u32, (key >> 32) as u32)? else {
            return None;
        };
        let start = u64::from(desc[10]) | u64::from(desc[11]) << 32;
        let end = {
            let mut placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
            let end = start.checked_add(placed.bytes(&resource, crate::resource::first_mip(&desc))?)?;
            if !matches!(placed.claim(index, start, end, &resource).0, Claim::Free | Claim::Own) {
                return None;
            }
            end
        };
        let len = usize::try_from(end - start).ok()?;
        (crate::texture::mapped_bytes(start as *const u8, len) > 0).then_some((resource, start, end))
    }

    /// [`HostHeap::texture_sweep`] of some slots, `handle` saying where the
    /// title's descriptors lie.
    /// `resume`: the slot of the part the put-backs start from (they go on
    /// from the part's start after the last slot); the slot after the last
    /// one put back is returned ([`sweep_resume`]).
    fn sweep_part(
        &self,
        slots: std::ops::Range<u32>,
        resume: u32,
        handle: impl Fn(u32) -> Option<u64>,
        frame: u64,
        put_back: bool,
        counts: &mut [u64; 10],
    ) -> u32 {
        use std::sync::atomic::Ordering::Relaxed;
        /// Put back per part, and only while fewer slots than this wait for
        /// an upload: a view the title forgot to clear may be among them,
        /// and the queue serves the newest first.
        const MOST: u32 = 128;
        const SHORT: usize = 4_000;
        let mut back = 0;
        // What waits for an upload, not the slots uploaded whole that only
        // wait for looks at their memory: counted with those, the queue
        // held some 17,000 slots and nothing was put back (10,378 views
        // with memory that nothing would bring up to date at the end of a
        // run, 899 put back in all). `VICTORY_SWEEP_GATE=all` counts them
        // as before.
        static ALL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let all = *ALL.get_or_init(|| std::env::var("VICTORY_SWEEP_GATE").as_deref() == Ok("all"));
        let mut waiting: Option<usize> = None;
        // The slots whose host slot does not show what the title's says.
        let mut unmade = Vec::new();
        {
            let made = self.made.lock().unwrap_or_else(|e| e.into_inner());
            // The memory is the title's, and a heap may be larger than what
            // of it is there: asked once per page.
            let mut page = (u64::MAX, false);
            for index in slots {
                let Some(handle) = handle(index) else {
                    break;
                };
                if page.0 != handle >> 12 {
                    page = (handle >> 12, crate::texture::committed(handle as *const u8, 12));
                }
                if !page.1 {
                    continue;
                }
                // SAFETY: a descriptor slot of the title's heap, there.
                let [tag, i, generation] = unsafe { (handle as *const [u32; 3]).read_unaligned() };
                if tag != VIEW_TAG || generation >> super::KIND_SHIFT == super::CONSTANT {
                    continue;
                }
                let key = u64::from(generation) << 32 | u64::from(i);
                let shown = made.get(index as usize).copied().unwrap_or(0);
                if shown == key {
                    counts[0] += 1;
                    counts[1] += 1;
                } else {
                    unmade.push((index, i, generation, shown != 0));
                }
            }
        }
        let pivot = unmade.partition_point(|&(index, ..)| index < resume);
        let mut next = resume;
        for &(index, i, generation, other) in unmade[pivot..].iter().chain(&unmade[..pivot]) {
            let Some(View::Placed(false, resource, desc)) = view_at(i, generation) else {
                continue;
            };
            if resource[0] == 1 {
                continue;
            }
            counts[0] += 1;
            let queued = {
                let textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
                textures.slots.contains_key(&index)
            };
            if queued {
                counts[2] += 1;
                continue;
            }
            if self.tabled_lately(index, frame) {
                counts[3] += 1;
                continue;
            }
            let start = u64::from(desc[10]) | u64::from(desc[11]) << 32;
            if !crate::texture::committed(start as *const u8, 1) {
                counts[4] += 1;
                continue;
            }
            let (claim, against) = {
                let mut placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
                let Some(bytes) = placed.bytes(&resource, crate::resource::first_mip(&desc)) else {
                    continue;
                };
                placed.claim(index, start, start.saturating_add(bytes), &resource)
            };
            counts[5 + claim as usize] += 1;
            counts[9] += u64::from(other);
            static SAID: [std::sync::atomic::AtomicU32; 4] = [const { std::sync::atomic::AtomicU32::new(0) }; 4];
            if SAID[claim as usize].fetch_add(1, Relaxed) < 12 {
                let (width, height, format) = (resource[4], resource[6], resource[8]);
                tracing::info!(
                    "a texture view nothing brings up to date: slot {index}, {claim:?}, {width}x{height} f{format} \
                     mips {} at {start:#x}, shows {}; against {against:x?}; its view {:x?}",
                    resource[7] >> 16,
                    if other { "another view" } else { "nothing" },
                    &desc[..10]
                );
            }
            if put_back && back < MOST && matches!(claim, Claim::Own | Claim::Free) {
                let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
                let queued = if all {
                    textures.slots.len()
                } else {
                    *waiting.get_or_insert_with(|| textures.uploads_waiting())
                };
                if queued < SHORT {
                    textures.written(index, frame, frame + 1, false);
                    back += 1;
                    next = index.saturating_add(1);
                    waiting = waiting.map(|n| n + 1);
                    SWEPT[10].fetch_add(1, Relaxed);
                }
            }
        }
        next
    }

    /// Takes the ranges of title memory written behind the guards' back
    /// since the last call ([`page_written`]) and finds the slots whose
    /// placed texture holds bytes of them ([`Spans::slots_holding`]). A
    /// slot whose texture was uploaded before the write shows the bytes
    /// from before: with `VICTORY_PAGE_WRITES` (`crate::watch::page_writes`,
    /// bit 2) it is due again [`PAGE_WRITE_AFTER`] frames on, as memory
    /// mapped under it would make it ([`HostHeap::texture_slots_remapped`]);
    /// its host copy exists, so the upload needs no room on the card. A
    /// slot still waiting for its first upload is left as it is: that
    /// upload takes the bytes that are there by then. Slots tables read
    /// are left to their draws. Counted either way; returns how many slots
    /// it made due.
    fn texture_slots_page_written(&self, frame: u64) -> usize {
        let ranges = std::mem::take(&mut *PAGE_WRITTEN.lock().unwrap_or_else(|e| e.into_inner()));
        if ranges.is_empty() {
            return 0;
        }
        self.slots_written_under(&ranges, frame, crate::watch::page_writes() & 2 != 0)[2]
    }

    /// [`HostHeap::texture_slots_page_written`] for `ranges` (start, end):
    /// the slots that held bytes of them, those of them uploaded before,
    /// and those made due (the latter, if `due`).
    fn slots_written_under(&self, ranges: &[(u64, u64)], frame: u64, due: bool) -> [usize; 3] {
        use std::sync::atomic::Ordering::Relaxed;
        let mut slots: Vec<u32> = {
            let placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
            ranges.iter().flat_map(|&(start, end)| placed.slots_holding(start, end)).collect()
        };
        slots.sort_unstable();
        slots.dedup();
        slots.retain(|&slot| !self.tabled_lately(slot, frame));
        let held = slots.len();
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        // Uploaded before: out of the queue, or in it after an upload (in
        // part, or whole and being looked at again).
        slots.retain(|slot| textures.slots.get(slot).is_none_or(|w| w.mapped > 0 || w.relook.is_some()));
        PAGE_WRITES[2].fetch_add(held as u64, Relaxed);
        PAGE_WRITES[3].fetch_add(slots.len() as u64, Relaxed);
        if !due {
            return [held, slots.len(), 0];
        }
        for &slot in &slots {
            textures.changed(slot, frame, frame + PAGE_WRITE_AFTER);
        }
        [held, slots.len(), slots.len()]
    }

    /// Whether host slot `index` is made from the very record the title
    /// has at `handle` now. Its host texture then exists (a slot keeps what
    /// it views), so bringing that texture up to date takes no memory of
    /// the card.
    pub(crate) fn texture_slot_made_from(&self, index: u32, handle: u64) -> bool {
        if index >= self.count {
            return false;
        }
        // SAFETY: the handle is a descriptor slot in title memory.
        let [tag, i, generation] = unsafe { (handle as *const [u32; 3]).read_unaligned() };
        let key = u64::from(generation) << 32 | u64::from(i);
        if tag != VIEW_TAG {
            return false;
        }
        // And the slot holds a host resource: a slot is also noted as made
        // from a view the host could not make (`make` gave nothing, the
        // slot is null and keeps nothing). The host copy of such a
        // texture is held by no slot, may have been let go, and would be
        // made anew here, with the card full. `made` first, then `kept`,
        // as `mirror` takes them.
        let made = self.made.lock().unwrap_or_else(|e| e.into_inner());
        made.get(index as usize).is_some_and(|&made| made != 0 && made == key)
            && self.kept_piece(index).is_some_and(|(kept, at)| kept[at].is_some())
    }

    /// Levels on demand (`crate::tiles`): makes again the host slots of the
    /// placed textures whose first level with memory changed since the
    /// last call ([`levels_changed`]), each from the view it was made
    /// from, so that it shows the texture from that level ([`make`]). Only
    /// slots that are made; a slot draws read through a table is made by
    /// its draw. Returns how many it made.
    /// shortcut: the starts are kept in one place for all heaps; the heap
    /// whose pass runs first takes them (a title has one view heap).
    /// Levels on demand (`crate::tiles`): the card has room, so the slots
    /// of up to [`Self::WAITING_A_PASS`] placed textures whose upload left
    /// a held level without tiles at least a second ago
    /// ([`levels_waiting`]) are due again from the next frame, as memory
    /// mapped under them would make them
    /// ([`HostHeap::texture_slots_remapped`]): the upload then gives the
    /// level its tiles. To be called only while the card has room: without
    /// it the upload would give nothing again. Slots tables read are left
    /// to their draws (`texture.rs` `upload` asks again for those). Returns
    /// how many slots it made due.
    /// shortcut: kept in one place for all heaps, as the starts below.
    pub(crate) fn texture_slots_waiting(&self, frame: u64) -> usize {
        if self.sampler {
            return 0;
        }
        let starts: Vec<u64> = {
            let mut waiting = WAITING.lock().unwrap_or_else(|e| e.into_inner());
            if waiting.is_empty() {
                return 0;
            }
            let starts: Vec<u64> = waiting
                .iter()
                .filter(|&(_, &since)| crate::host::long_ago(since, 1000, 2))
                .map(|(&start, _)| start)
                .take(Self::WAITING_A_PASS)
                .collect();
            for start in &starts {
                waiting.remove(start);
            }
            starts
        };
        if starts.is_empty() {
            return 0;
        }
        let mut slots: Vec<u32> = {
            let placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
            starts.iter().flat_map(|&start| placed.slots_at(start)).collect()
        };
        slots.sort_unstable();
        slots.dedup();
        slots.retain(|&slot| !self.tabled_lately(slot, frame));
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        for &slot in &slots {
            textures.changed(slot, frame, frame + 1);
        }
        crate::tiles::STATS[10].fetch_add(slots.len() as u64, std::sync::atomic::Ordering::Relaxed);
        slots.len()
    }

    /// Textures a pass asks again for their levels
    /// ([`HostHeap::texture_slots_waiting`]): the first ones given tiles
    /// may take the room the others were asked for, and an upload that
    /// gives nothing is a whole texture staged and copied for nothing.
    const WAITING_A_PASS: usize = 128;

    pub(crate) fn texture_views_remade(&self) -> usize {
        if self.sampler {
            return 0;
        }
        let starts = std::mem::take(&mut *RESHOWN.lock().unwrap_or_else(|e| e.into_inner()));
        if starts.is_empty() {
            return 0;
        }
        let slots: Vec<u32> = {
            let placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
            starts.iter().flat_map(|&start| placed.slots_at(start)).collect()
        };
        let made = self.made.lock().unwrap_or_else(|e| e.into_inner());
        let mut remade = 0;
        for slot in slots {
            let Some(&key) = made.get(slot as usize) else {
                continue;
            };
            if key == 0 {
                continue;
            }
            let Some(view) = view_at(key as u32, (key >> 32) as u32) else {
                continue;
            };
            if let Some(kept) = make(&view, self.cpu(slot)) {
                self.keep(slot, kept);
                remade += 1;
            }
        }
        remade
    }

    /// The memory of the texture title slot `index` views is not mapped
    /// (any more): a host slot made from it becomes null again, so that
    /// the host copy can be let go (`crate::resource::let_go_idle`; a slot
    /// keeps what it views). True if the slot was made.
    /// Where the title's writes of slots and of the queue stand now: what a
    /// material record read now is checked against when the card hands it
    /// back (`crate::material_demand`). Makes the per-slot write numbers
    /// the first time.
    pub(crate) fn material_write_stamp(&self) -> crate::material_demand::Mark {
        self.material_epochs
            .get_or_init(|| (0..self.count).map(|_| std::sync::atomic::AtomicU64::new(0)).collect());
        crate::material_demand::Mark {
            write: self.textures.lock().unwrap_or_else(|e| e.into_inner()).writes,
            descriptor: self.material_clock.load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Slot `index`, which a material record the card read at `stamp`
    /// names (`crate::material_demand`): its entry in the queue if it is
    /// due for an upload and its write is no newer than the read (the pass
    /// then takes it sooner); or, for a slot that shows nothing and is not
    /// in the queue, a view of a typed block-compressed 2D texture whose
    /// memory is all there and no other view claims, put into the queue
    /// now. Never one whose memory a texture the card writes, or the host
    /// copied into, lies over: an upload from title memory would go over
    /// what the card has. None if neither.
    pub(crate) fn demand_material_texture(
        &self,
        heap: &super::Heap,
        index: u32,
        stamp: crate::material_demand::Mark,
        frame: u64,
    ) -> Option<(u32, u64)> {
        let handle = heap.handle(index)?;
        // Held throughout, as a title write holds it.
        let made = self.made.lock().unwrap_or_else(|e| e.into_inner());
        let key = *made.get(index as usize)?;
        if self.material_epochs.get()?.get(index as usize)?.load(std::sync::atomic::Ordering::Relaxed) > stamp.descriptor {
            return None;
        }
        let View::Placed(false, resource, desc) = super::checked_view(handle)? else {
            return None;
        };
        if resource[0] == 1 {
            return None;
        }
        let start = u64::from(desc[10]) | u64::from(desc[11]) << 32;
        let (len, claim) = {
            let mut placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
            let len = placed.bytes(&resource, crate::resource::first_mip(&desc))?;
            let end = start.checked_add(len)?;
            if len == 0 || start == 0 {
                return None;
            }
            (len, placed.claim(index, start, end, &resource).0)
        };
        let end = start + len;
        {
            let textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(wait) = textures.slots.get(&index) {
                return (claim == Claim::Own && wait.relook.is_none() && wait.due <= frame && wait.write <= stamp.write)
                    .then_some((index, wait.write));
            }
        }
        // What the record's fields feed is an image instruction's
        // descriptor: only a typed block-compressed format matching its
        // resource, never a typeless view.
        let typed_bc = matches!(resource[8], 71 | 72 | 74 | 75 | 77 | 78 | 80 | 81 | 83 | 84 | 95 | 96 | 98 | 99);
        if key != 0
            || !matches!(claim, Claim::Own | Claim::Free)
            || resource[0] != 3
            || resource[7] as u16 != 1
            || !typed_bc
            || resource[9] != 1
            || resource[12] & 15 != 0
            || desc[0] != resource[8]
            || desc[1] != 4
            || desc[4] >= resource[7] >> 16
            || crate::texture::mapped_bytes(start as *const u8, usize::try_from(len).ok()?) != len
            || crate::resource::writable_texture_overlaps(start, end)
            || crate::texture::host_written_overlaps(start, end)
        {
            return None;
        }
        self.placed.lock().unwrap_or_else(|e| e.into_inner()).note(index, start, end, &resource, false);
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        if textures.slots.contains_key(&index) {
            return None;
        }
        textures.written(index, frame, frame, false);
        let wait = textures.slots.get_mut(&index)?;
        wait.recovered = true;
        let write = wait.write;
        drop(textures);
        drop(made);
        static REVIVED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = REVIVED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if n <= 16 || n.is_power_of_two() {
            tracing::info!(
                frame,
                slot = index,
                first_mip = desc[4],
                bytes = len,
                revived = n,
                format = desc[0],
                "a texture slot a material record the card read names, null and out of the queue, goes into it"
            );
        }
        Some((index, write))
    }

    /// Tests: slot `index` out of the queue of textures picked by index.
    #[cfg(test)]
    pub(crate) fn textures_remove(&self, index: u32) {
        self.textures.lock().unwrap_or_else(|e| e.into_inner()).remove(index);
    }

    /// Whether slot `index`, as the pass was handed it (`write`), went into
    /// the queue for a material record ([`HostHeap::demand_material_texture`]).
    pub(crate) fn texture_slot_recovered(&self, index: u32, write: u64) -> bool {
        self.textures.lock().unwrap_or_else(|e| e.into_inner()).same(index, write).is_some_and(|w| w.recovered)
    }

    /// For a slot put into the queue for a material record: whether its
    /// upload may go on now (`mapped` of its `len` bytes from `start` are
    /// there). Between the record and the upload the pass may have waited,
    /// a texture the card writes may have come over the same memory, or
    /// streaming may have taken a level out.
    pub(crate) fn material_upload_allowed(&self, index: u32, write: u64, start: u64, len: u64, mapped: u64) -> bool {
        if !self.texture_slot_recovered(index, write) {
            return false;
        }
        start.checked_add(len).is_some_and(|end| {
            mapped == len
                && !crate::resource::writable_texture_overlaps(start, end)
                && !crate::texture::host_written_overlaps(start, end)
        })
    }

    /// [`HostHeap::mirror`] for the texture pass (with
    /// `crate::material_demand` or [`displaced_recovery`] on): only while
    /// the pass's entry (`write`) is still the queue's, and a slot a newer
    /// view had made stale only once its span is its own again (a sweep may
    /// have put it back from what it saw before; a newer view takes it
    /// away again under `made`).
    pub(crate) fn mirror_queued(&self, index: u32, handle: u64, write: u64) -> Option<View> {
        self.textures.lock().unwrap_or_else(|e| e.into_inner()).same(index, write)?;
        if displaced_recovery() {
            // SAFETY: the handle is a descriptor slot in title memory.
            let [tag, i, generation] = unsafe { (handle as *const [u32; 3]).read_unaligned() };
            let key = u64::from(generation) << 32 | u64::from(i);
            if tag == VIEW_TAG
                && let Some(View::Placed(false, resource, desc)) = view_at(i, generation)
            {
                let mut placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
                if placed.displaced.get(&index) == Some(&key) {
                    let start = u64::from(desc[10]) | u64::from(desc[11]) << 32;
                    let span = placed
                        .bytes(&resource, crate::resource::first_mip(&desc))
                        .map(|bytes| (start, start.saturating_add(bytes)));
                    if span.is_none() || placed.by_slot.get(&index) != span.as_ref() {
                        return None;
                    }
                }
            }
        }
        self.mirror(index, handle, true)
    }

    pub(crate) fn texture_slot_gone(&self, index: u32, write: u64) -> bool {
        let mut made = self.made.lock().unwrap_or_else(|e| e.into_inner());
        if made.get(index as usize).is_none_or(|&key| key == 0) {
            return false;
        }
        // Mapping was checked outside these locks. A newer descriptor may
        // have been mirrored, or a table read may have removed the entry.
        // Mirror takes the made lock before updating the texture queue.
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        if textures.same(index, write).is_none() {
            GONE_REFUSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        self.unmake(&mut made, index);
        STALE[4].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        true
    }

    /// The stale slots of this heap left showing their view
    /// ([`stale_after_flights`]) whose submissions have finished: made null
    /// now, if they still hold the view they held. `made` is this heap's,
    /// locked by the caller.
    fn stale_later_due(&self, made: &mut [u64]) {
        let due = {
            let mut later = STALE_LATER.lock().unwrap_or_else(|e| e.into_inner());
            let due = stale_due(&mut later, std::ptr::from_ref(self) as usize, stale_clock().1);
            if later.is_empty() {
                STALE_LATER_ANY.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            due
        };
        for (slot, key) in due {
            if made.get(slot as usize) == Some(&key) {
                self.unmake(made, slot);
                BAKE_SLOTS[3].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// [`HostHeap::stale_later_due`] from outside a mirror (the texture
    /// pass): nothing if nothing waits.
    pub(crate) fn stale_later(&self) {
        if !STALE_LATER_ANY.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let mut made = self.made.lock().unwrap_or_else(|e| e.into_inner());
        self.stale_later_due(&mut made);
    }

    /// Makes host slot `index` null again, as it is before its view is
    /// made: a null view of the kind it held.
    fn unmake(&self, made: &mut [u64], index: u32) {
        let Some(key) = made.get_mut(index as usize).map(std::mem::take) else {
            return;
        };
        if key == 0 {
            return;
        }
        self.made_fast_set(index, 0);
        match view_at(key as u32, (key >> 32) as u32) {
            Some(view) => null_for(&view, self.cpu(index)),
            None => null_view(self.cpu(index)),
        }
        self.keep(index, None);
    }

    /// Title slot `index` is a table's ([`HostHeap::tabled`]). `read`: a
    /// draw read it through the table and brought what it views up to
    /// date, so the slot leaves the queue of textures picked by index; the
    /// caller makes the host slot in place ([`HostHeap::mirror`]), as the
    /// pass would have, for a shader that picks it by index all the same.
    pub(crate) fn texture_slot_tabled(&self, index: u32, read: bool) {
        self.tabled_in(index, read, crate::frame::current());
    }

    /// [`HostHeap::texture_slot_tabled`] in frame `frame`.
    fn tabled_in(&self, index: u32, read: bool, frame: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        let Some(stamp) = self.tabled.get(index as usize) else {
            return;
        };
        let now = (frame as u32).wrapping_add(1) & !TABLED_READ;
        let old = stamp.load(Relaxed);
        if !read {
            if old & !TABLED_READ != now {
                stamp.store(now, Relaxed);
            }
            return;
        }
        // Once per slot and frame: tables are read by every draw.
        if old != now | TABLED_READ {
            stamp.store(now | TABLED_READ, Relaxed);
            if tabled_out() {
                let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
                if textures.slots.remove(&index).is_some() {
                    TABLED_WRITES[1].fetch_add(1, Relaxed);
                }
            }
        }
    }

    /// Whether title slot `index` was a table's within the last
    /// [`TABLED_FRAMES`] frames before `frame` (never, if such slots are
    /// to be queued as any other).
    fn tabled_lately(&self, index: u32, frame: u64) -> bool {
        let stamp = self.tabled_stamp(index) & !TABLED_READ;
        let now = (frame as u32).wrapping_add(1) & !TABLED_READ;
        tabled_out() && stamp != 0 && u64::from(now.wrapping_sub(stamp)) <= TABLED_FRAMES
    }

    /// What `tabled` holds for title slot `index`; 0: never a table's.
    fn tabled_stamp(&self, index: u32) -> u32 {
        self.tabled
            .get(index as usize)
            .map_or(0, |s| s.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// The queue's part of a write of title slot `index` in `frame`:
    /// `texture`, whether it now holds a view of a texture the title may
    /// fill; `seen` as for [`HostHeap::note_buffer`].
    fn queue_texture(&self, index: u32, texture: bool, seen: bool, frame: u64) {
        let stamp = if tabled_out() { self.tabled_stamp(index) } else { 0 };
        let now = (frame as u32).wrapping_add(1) & !TABLED_READ;
        // A table's slot lately: the draw that reads it uploads its
        // texture, so its write waits for that draw.
        let lately = stamp != 0 && u64::from(now.wrapping_sub(stamp & !TABLED_READ)) <= TABLED_FRAMES;
        // Made now by a draw of this frame that read it through a table:
        // up to date.
        let read_now = seen && stamp == now | TABLED_READ;
        let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        if texture && !read_now {
            let due = if lately && !seen {
                TABLED_WRITES[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                frame + TABLED_GRACE
            } else {
                0
            };
            textures.written(index, frame, due, seen);
        } else {
            textures.remove(index);
        }
    }

    /// Records what title slot `index` now holds for shaders that pick
    /// buffers from the heap: its element stride in the table, and whether
    /// it is a buffer view at all.
    /// `seen`: the slot was not written now, a draw or a pass made its
    /// host slot from what it held.
    /// Returns the title slots whose placed texture view the write made
    /// stale ([`Spans`]): out of the queue of textures already, their host
    /// slots are the caller's to make null (it holds `made`).
    fn note_buffer(&self, index: u32, view: Option<&View>, seen: bool) -> Vec<u32> {
        if self.sampler {
            return Vec::new();
        }
        // Measured for the texture side of the same gap (textures a shader
        // picks by a computed index are neither made, if placed, nor
        // uploaded): slots written with a placed texture view, and with a
        // view of a texture object.
        let texture = match view {
            Some(View::Placed(false, resource, _)) if resource[0] != 1 => {
                PLACED_TEXTURE_SLOTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                true
            }
            Some(View::Shader(r, _)) if crate::resource::buffer_bytes(*r as crate::object::This).is_none() => {
                OBJECT_TEXTURE_SLOTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                true
            }
            _ => false,
        };
        self.queue_texture(index, texture, seen, crate::frame::current());
        // The memory a placed texture view describes. Not for a slot
        // tables read (thousands of writes a frame, each read by its
        // draw): no pass looks at it, and the same texture's view in a
        // slot shaders pick by index says what it says.
        let placed_view = match view {
            Some(View::Placed(false, resource, desc))
                if resource[0] != 1 && !self.tabled_lately(index, crate::frame::current()) =>
            {
                let first = crate::resource::first_mip(desc);
                Some((resource, u64::from(desc[10]) | u64::from(desc[11]) << 32, first))
            }
            _ => None,
        };
        let stale = {
            use std::sync::atomic::Ordering::Relaxed;
            let mut placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
            let span = placed_view.and_then(|(resource, start, first)| {
                Some((resource, start, start.saturating_add(placed.bytes(resource, first)?), first))
            });
            match span {
                Some((resource, start, end, first)) => {
                    SPANNED[0].fetch_min(start, Relaxed);
                    SPANNED[1].fetch_max(end, Relaxed);
                    placed.rule = stale_rule();
                    let stale = placed.note(index, start, end, resource, !seen);
                    placed.shows(index, first);
                    stale
                }
                None => {
                    placed.forget(index);
                    Vec::new()
                }
            }
        };
        if !stale.is_empty() {
            STALE[0].fetch_add(stale.len() as u64, std::sync::atomic::Ordering::Relaxed);
            let mut textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
            for &slot in &stale {
                textures.remove(slot);
            }
        }
        let stride = view.and_then(view_stride);
        if let Some((_, _, data)) = &self.strides
            && index < victory_shader::emit::SLOT_STRIDES_COUNT
        {
            // Constant buffer views marked: programs read no other
            // non-buffer slot as one (`SLOT_STRIDE_CONSTANT`).
            let entry = match (stride, view) {
                (Some(s), _) => s,
                (None, Some(View::Constant(..))) if victory_shader::emit::constant_marks() => {
                    victory_shader::emit::SLOT_STRIDE_CONSTANT
                }
                _ => 0,
            };
            // SAFETY: inside the mapping; shaders read it while we write (a
            // dword, either value is one the slot had).
            unsafe { (*data as *mut u32).add(index as usize).write_volatile(entry) };
        }
        if let Some((_, _, data)) = &self.addresses
            && index < victory_shader::emit::SLOT_STRIDES_COUNT
        {
            let address = view.map_or(0, view_address);
            // SAFETY: inside the mapping (two dwords a slot); a shader that
            // reads while we write may see half of the new address, as a
            // console shader may read a descriptor the CPU is writing.
            unsafe {
                let at = (*data as *mut u32).add(2 * index as usize);
                at.write_volatile(address as u32);
                at.add(1).write_volatile((address >> 32) as u32);
            }
        }
        // The pass over buffer views takes the range of a slot it makes
        // itself: noted, not marked changed for the next pass, which looked
        // at every slot it made a second time (half of the 50,000 to
        // 140,000 slots a frame in the open world).
        // `VICTORY_PASS_REVISITS=1`: marked, as before.
        if SYNC_NOTING.get() {
            SYNC_NOTED.set(self.buffers.noted(index, stride.is_some()));
        } else if in_heap_pass() && !pass_revisits() {
            self.buffers.noted(index, stride.is_some());
        } else {
            self.buffers.written(index, stride.is_some());
        }
        stale
    }

    /// Gives back `n` dynamic slots from `first` (an index [`dynamic`]
    /// returned).
    ///
    /// [`dynamic`]: HostHeap::dynamic
    pub(crate) fn release(&self, first: u32, n: u32) {
        let Some(at) = first.checked_sub(self.count) else {
            return;
        };
        let mut guard = self.next.lock().unwrap_or_else(|e| e.into_inner());
        let end = (at + n).min(self.dynamic) as usize;
        if let Some(slots) = guard.1.get_mut(at as usize..end) {
            let given = slots.iter().filter(|taken| **taken).count() as u32;
            self.free.fetch_add(given, std::sync::atomic::Ordering::Relaxed);
            self.too_long.store(u32::MAX, std::sync::atomic::Ordering::Relaxed);
            slots.fill(false);
        }
    }

    /// Makes slot `index` (a dynamic one) from `view`; a null view if the
    /// host cannot.
    pub(crate) fn make_at(&self, index: u32, view: &View) {
        let kept = make(view, self.cpu(index));
        if kept.is_none() {
            null_for(view, self.cpu(index));
        }
        self.keep(index, kept.flatten());
    }

    /// Makes slot `index` read nothing: a null view, or a plain sampler.
    pub(crate) fn make_null(&self, index: u32) {
        if self.sampler {
            self.null_sampler(index);
        } else {
            null_view(self.cpu(index));
        }
        self.keep(index, None);
    }

    /// The piece of [`HostHeap::kept`] slot `index` is in, locked, and
    /// where in it the slot is; `None` past the heap.
    fn kept_piece(&self, index: u32) -> Option<(std::sync::MutexGuard<'_, Vec<Option<ID3D12Resource>>>, usize)> {
        if index >= self.count + self.dynamic {
            return None;
        }
        let piece = self.kept.get((index / KEPT_PIECE) as usize)?;
        Some((piece.lock().unwrap_or_else(|e| e.into_inner()), (index % KEPT_PIECE) as usize))
    }

    /// The host resource slot `index` shows (tests).
    #[cfg(test)]
    fn kept_at(&self, index: u32) -> Option<ID3D12Resource> {
        self.kept_piece(index).and_then(|(kept, at)| kept[at].clone())
    }

    /// Slot `index` now views `resource` (or nothing of the host's). Every
    /// slot write comes here: its mark table entry follows.
    fn keep(&self, index: u32, resource: Option<ID3D12Resource>) {
        self.set_mark(index, resource.as_ref());
        // Whatever the slot showed, it is not known to be that view now
        // (the caller notes it again if it is: [`HostHeap::note_shown`]).
        if let Some(sig) = self.shown.lock().unwrap_or_else(|e| e.into_inner()).get_mut(index as usize) {
            *sig = 0;
        }
        if let Some((mut kept, at)) = self.kept_piece(index) {
            let slot = &mut kept[at];
            // A bake named it lately: counted if what it shows changes
            // ([`bake_slot_changed`]).
            let changed = bake_watch_on()
                && slot.as_ref().is_some_and(|old| {
                    resource.as_ref().is_none_or(|new| {
                        windows::core::Interface::as_raw(new) != windows::core::Interface::as_raw(old)
                    })
                });
            let nulled = resource.is_none();
            let old = std::mem::replace(slot, resource);
            drop(kept);
            // A running submission may still read the old resource through
            // this slot. (Let go outside the lock: that takes another, or
            // frees a host resource.)
            if let Some(old) = old {
                crate::host::retire(old);
            }
            if changed {
                bake_slot_changed(self, index, nulled);
            }
        }
    }

    /// Makes slot `index` (a dynamic one) a raw buffer view of `len` bytes
    /// of `buffer` from byte `offset` (a multiple of 16): a root SRV with a
    /// size, whose reads past it return zeros instead of reaching whatever
    /// the host placed after the buffer.
    pub(crate) fn raw_at(&self, index: u32, buffer: &ID3D12Resource, offset: u64, len: u64) {
        let Some(host) = host() else { return };
        let elements = (len / 4).clamp(1, u64::from(u32::MAX)) as u32;
        let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_R32_TYPELESS,
            ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Buffer: D3D12_BUFFER_SRV {
                    FirstElement: offset / 4,
                    NumElements: elements,
                    StructureByteStride: 0,
                    Flags: D3D12_BUFFER_SRV_FLAG_RAW,
                },
            },
        };
        // SAFETY: a slot of our heap and a live buffer covering the range.
        unsafe {
            host.device
                .CreateShaderResourceView(buffer, Some(&desc), self.cpu(index))
        };
        self.keep(index, Some(buffer.clone()));
    }

    /// Makes slot `index` (a dynamic one) a raw buffer view that reads
    /// zeros.
    pub(crate) fn null_raw_at(&self, index: u32) {
        null_buffer_view(self.cpu(index));
        self.keep(index, None);
    }

    /// Makes slot `index` (a dynamic one) a raw unordered access view of
    /// `len` bytes of `buffer` from byte `offset` (a multiple of 16): a
    /// root UAV with a size, whose accesses past it do nothing instead of
    /// reaching whatever the host placed after the buffer.
    pub(crate) fn raw_uav_at(&self, index: u32, buffer: &ID3D12Resource, offset: u64, len: u64) {
        let Some(host) = host() else { return };
        let elements = (len / 4).clamp(1, u64::from(u32::MAX)) as u32;
        let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
            Format: DXGI_FORMAT_R32_TYPELESS,
            ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
            Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                Buffer: D3D12_BUFFER_UAV {
                    FirstElement: offset / 4,
                    NumElements: elements,
                    StructureByteStride: 0,
                    CounterOffsetInBytes: 0,
                    Flags: D3D12_BUFFER_UAV_FLAG_RAW,
                },
            },
        };
        // SAFETY: a slot of our heap and a live buffer covering the range.
        unsafe {
            host.device.CreateUnorderedAccessView(
                buffer,
                None::<&ID3D12Resource>,
                Some(&desc),
                self.cpu(index),
            )
        };
        self.keep(index, Some(buffer.clone()));
    }

    /// Makes slot `index` (a dynamic one) a raw unordered access view of
    /// nothing: reads give zeros, writes are dropped.
    pub(crate) fn null_raw_uav_at(&self, index: u32) {
        self.keep(index, None);
        let Some(host) = host() else { return };
        let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
            Format: DXGI_FORMAT_R32_TYPELESS,
            ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
            Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                Buffer: D3D12_BUFFER_UAV {
                    NumElements: 1,
                    Flags: D3D12_BUFFER_UAV_FLAG_RAW,
                    ..Default::default()
                },
            },
        };
        // SAFETY: a slot of our heap; a null resource is allowed with a desc.
        unsafe {
            host.device.CreateUnorderedAccessView(
                None::<&ID3D12Resource>,
                None::<&ID3D12Resource>,
                Some(&desc),
                self.cpu(index),
            )
        };
    }

    /// The shader-visible handle of slot `index`.
    pub(crate) fn gpu_handle(&self, index: u32) -> D3D12_GPU_DESCRIPTOR_HANDLE {
        D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: self.gpu + u64::from(index) * u64::from(self.step),
        }
    }

    /// Notes in [`HostHeap::made_fast`] that slot `index` is made from
    /// record `key` (0: from none). The caller holds `made`'s lock.
    fn made_fast_set(&self, index: u32, key: u64) {
        if let Some(fast) = self.made_fast.get(index as usize) {
            fast.store(key, std::sync::atomic::Ordering::Release);
        }
    }

    /// Makes slot `index` (a dynamic one) a constant buffer view of host
    /// memory (the list's own, which it holds).
    pub(crate) fn constant_at(&self, index: u32, gpu: u64, size: u32) {
        self.keep(index, None);
        let Some(host) = host() else { return };
        let desc = D3D12_CONSTANT_BUFFER_VIEW_DESC {
            BufferLocation: gpu,
            SizeInBytes: constant_bytes(size),
        };
        // SAFETY: a slot of our heap; the memory is the caller's.
        unsafe {
            host.device
                .CreateConstantBufferView(Some(&desc), self.cpu(index))
        };
    }

    fn cpu(&self, index: u32) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.cpu + index as usize * self.step as usize,
        }
    }

    /// For a slot on a page of the title's heap that changed with no call
    /// (`super::sync_heaps`: the title copies records into its heap as
    /// bytes): [`HostHeap::mirror`], if the `bytes` bytes of the record at
    /// `handle` are not the ones this last saw there. True if it did.
    ///
    /// The page says only that something on it was written, and `mirror`
    /// by itself would take every buffer or placed-texture record on it for
    /// newly written each time (it leaves such a slot unmade until a draw
    /// or a pass uses it): the texture queue's entry for the slot would
    /// start again, and the buffer pass would look at 2,048 slots for one
    /// that changed.
    /// `record`: the record's bytes as the caller read them (through the
    /// kernel, which fails on a page the title unmapped meanwhile where a
    /// read in place would fault: the title unmaps heap pages too).
    pub(crate) fn mirror_copied(&self, index: u32, handle: u64, record: &[u8]) -> bool {
        if index >= self.count {
            return false;
        }
        // A slot draws read through a table lately is left to its draws,
        // as the passes over slots picked by index leave it
        // (`texture_slots_remapped`, `slots_written_under`): the title
        // writes its per-draw tables into the same heap every frame, and
        // each draw makes what its table shows in a slot of its own. Those
        // were most of the 26,000 to 43,000 records a frame this pass
        // mirrored in run 200 and in the play session after it, each then
        // made and brought up to date once more by the pass over buffer
        // views (some 50 ms a frame). `VICTORY_HEAP_SYNC_TABLES=1`: they
        // are mirrored too, as in run 200.
        if !sync_tables() && self.tabled_lately(index, crate::frame::current()) {
            COPIED_TABLED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        let n = record.len();
        // Never 0, which is "never looked at". (A multiply-and-fold over
        // the record's words: BLAKE3 here was most of the 35 to 42 ms a
        // frame this pass took over some 250,000 to 300,000 records, run
        // 200.)
        let print = record_print(record) | 1;
        {
            let mut copied = self.copied.lock().unwrap_or_else(|e| e.into_inner());
            if copied.is_empty() {
                copied.resize(self.count as usize, 0);
            }
            let Some(last) = copied.get_mut(index as usize) else {
                return false;
            };
            // A slot never looked at that holds nothing: as it was.
            if *last == print || (*last == 0 && record[..n].iter().all(|&b| b == 0)) {
                return false;
            }
            *last = print;
        }
        self.mirror_as(index, handle, false, sync_makes());
        true
    }

    /// [`HostHeap::mirror_copied`] for every record of one changed page of
    /// the title's heap (`super::sync_heaps`): title slots `slots`, the
    /// record of slot `i` at title address `start + i * step`, and `page`
    /// the bytes read from title address `from` on. Returns the records
    /// looked at and those mirrored.
    ///
    /// The same records are mirrored as one by one. What differs is the
    /// work for the others, which are nearly all of them (some 330,000
    /// looked at a frame for 14,000 mirrored, 16 ms, run 225): the lock of
    /// the fingerprints is taken once for the page and not once for each
    /// record, and a record whose fingerprint is the one last seen is done
    /// with before the question whether its slot was a table's lately. So
    /// the count of slots "left to the draws" is now of records that
    /// changed, not of every record on the page. The slots to mirror are
    /// made after the page's walk, in slot order as before.
    pub(crate) fn mirror_copied_page(
        &self,
        slots: std::ops::Range<u64>,
        (start, step): (u64, u64),
        from: usize,
        page: &[u8],
    ) -> (u64, u64) {
        let (frame, tables) = (crate::frame::current(), sync_tables());
        let mut looked = 0u64;
        let mut due: Vec<(u32, u64)> = Vec::new();
        {
            let mut copied = self.copied.lock().unwrap_or_else(|e| e.into_inner());
            if copied.is_empty() {
                copied.resize(self.count as usize, 0);
            }
            for index in slots {
                let handle = start + index * step;
                // A record that straddles the range's ends is left to the
                // range that holds its first byte.
                let Some(at) = (handle as usize).checked_sub(from) else {
                    continue;
                };
                let Some(record) = page.get(at..at + step as usize) else {
                    continue;
                };
                looked += 1;
                // None past the host heap's slots, as `mirror_copied`.
                let Some(last) = copied.get_mut(index as usize) else {
                    continue;
                };
                // Never 0, which is "never looked at".
                let print = record_print(record) | 1;
                // The record last seen, or a slot never looked at that
                // holds nothing: as it was.
                if *last == print || (*last == 0 && record.iter().all(|&b| b == 0)) {
                    continue;
                }
                // A table's slot lately: left to its draws, and looked at
                // again next time.
                if !tables && self.tabled_lately(index as u32, frame) {
                    COPIED_TABLED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
                *last = print;
                due.push((index as u32, handle));
            }
        }
        for &(index, handle) in &due {
            self.mirror_as(index, handle, false, sync_makes());
        }
        (looked, due.len() as u64)
    }

    /// Makes host slot `index` from the record the title has at `handle`
    /// if it changed since, and returns the view recorded there. Buffer
    /// views are made only for draws (`for_draw`), which also copy their
    /// data: a host copy of a buffer is made on first use (and at the
    /// mirroring of copied records, [`sync_makes`]).
    pub(crate) fn mirror(&self, index: u32, handle: u64, for_draw: bool) -> Option<View> {
        self.mirror_as(index, handle, for_draw, false)
    }

    /// [`HostHeap::mirror`]; `make_buffers`: a buffer view (as
    /// [`buffer_view_kind`] tells) is made now even when not `for_draw`,
    /// as a draw would make it ([`sync_makes`]).
    fn mirror_as(&self, index: u32, handle: u64, for_draw: bool, make_buffers: bool) -> Option<View> {
        if index >= self.count {
            // Diagnostic (NVIDIA heap cap): the highest title slot past the
            // host heap seen so far.
            static HIGHEST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if HIGHEST.fetch_max(index, std::sync::atomic::Ordering::Relaxed) < index {
                tracing::warn!(
                    index,
                    count = self.count,
                    for_draw,
                    "title slot past the host heap"
                );
            }
            return None;
        }
        // SAFETY: the handle is a descriptor slot in title memory.
        let [tag, i, generation] = unsafe { (handle as *const [u32; 3]).read_unaligned() };
        // Generations start at 1, so a made slot's key is never 0.
        let key = u64::from(generation) << 32 | u64::from(i);
        // A slot made from the very record it holds (nearly every mirroring,
        // draws' and passes' alike): nothing to make, and `made`'s lock is
        // not taken. Not while stale slots wait to be made null
        // ([`stale_after_flights`]): a write of one of them makes it anew.
        if tag == VIEW_TAG
            && key != 0
            && mirror_fast()
            && !STALE_LATER_ANY.load(std::sync::atomic::Ordering::Relaxed)
            && self.made_fast[index as usize].load(std::sync::atomic::Ordering::Acquire) == key
            && let Some(view) = view_at(i, generation)
        {
            if !for_draw && let Some(epochs) = self.material_epochs.get() {
                let sequence = self.material_clock.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(epoch) = epochs.get(index as usize) {
                    epoch.store(sequence, std::sync::atomic::Ordering::Relaxed);
                }
            }
            if for_draw && in_heap_pass() {
                SAME_VIEWS[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            MIRROR_FAST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Some(view);
        }
        let mut made = self.made.lock().unwrap_or_else(|e| e.into_inner());
        let heap_key = std::ptr::from_ref(self) as usize;
        if STALE_LATER_ANY.load(std::sync::atomic::Ordering::Relaxed) {
            // The slot written now is the title's again: whatever stale
            // view it was left showing is made anew from its record, as
            // if it had been made null when found stale.
            let mut later = STALE_LATER.lock().unwrap_or_else(|e| e.into_inner());
            let before = later.len();
            later.retain(|e| !(e.0 == heap_key && e.1 == index));
            if later.len() != before
                && let Some(shown) = made.get_mut(index as usize)
            {
                *shown = 0;
                self.made_fast_set(index, 0);
            }
            drop(later);
            self.stale_later_due(&mut made);
        }
        // A write of the title's (`crate::material_demand`): numbered, while
        // material records are read.
        if !for_draw && let Some(epochs) = self.material_epochs.get() {
            let sequence = self.material_clock.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if let Some(epoch) = epochs.get(index as usize) {
                epoch.store(sequence, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let view = (tag == VIEW_TAG).then(|| view_at(i, generation)).flatten();
        let Some(view) = view else {
            // Not a view of ours (never written, or of a resource the
            // title released): nothing to read.
            if made[index as usize] != 0 {
                made[index as usize] = 0;
                self.made_fast_set(index, 0);
                self.make_null(index);
                let _ = self.note_buffer(index, None, false);
            } else if for_draw
                || (displaced_recovery()
                    && !self.sampler
                    && self.placed.lock().unwrap_or_else(|e| e.into_inner()).by_slot.contains_key(&index))
            {
                // Left for later and stale since: out of the sets and the stride table.
                // (With `displaced_recovery`, a cleared slot never made
                // gives up the memory it claimed too: a recovery waits for
                // no view to claim it.)
                let _ = self.note_buffer(index, None, false);
            }
            return None;
        };
        if made[index as usize] != key {
            // A new record that names the view the slot shows already (the
            // title writes its per-frame records again, with the same
            // contents, under new names): the slot stays as it is.
            let sig = if self.sampler { None } else { same_view_signature(&view) };
            if let Some(sig) = sig {
                if self.shows_same(index, sig, &view) {
                    made[index as usize] = key;
                    self.made_fast_set(index, key);
                    SAME_VIEWS[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Some(view);
                }
                SAME_VIEWS[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            #[cfg(feature = "diagnostics")]
            let making = std::time::Instant::now();
            let make_now = make_buffers && buffer_view_kind(&view);
            // Made here and its range given to the pass ([`sync_ranges`]):
            // the books note the slot without marking it changed.
            let gives_range = make_now && sync_ranges();
            SYNC_NOTING.set(gives_range);
            SYNC_NOTED.set(false);
            // Views of other textures in the memory this one describes
            // are stale: their host slots read nothing from here on, and
            // let go of the host copies they held.
            let stale_slots = self.note_buffer(index, Some(&view), for_draw);
            SYNC_NOTING.set(false);
            if displaced_recovery() && !stale_slots.is_empty() {
                let mut placed = self.placed.lock().unwrap_or_else(|e| e.into_inner());
                for &slot in &stale_slots {
                    let shown = made.get(slot as usize).copied().unwrap_or(0);
                    if slot != index && shown != 0 {
                        placed.displaced.insert(slot, shown);
                    }
                }
            }
            // With submissions in the air, left showing their view until
            // those have finished ([`stale_after_flights`]).
            let (started, finished) = stale_clock();
            let defer = stale_after_flights() && started > finished;
            for stale in stale_slots {
                if stale == index {
                    continue;
                }
                match made.get(stale as usize) {
                    Some(&key) if defer && key != 0 => {
                        let mut later = STALE_LATER.lock().unwrap_or_else(|e| e.into_inner());
                        later.retain(|e| !(e.0 == heap_key && e.1 == stale));
                        later.push((heap_key, stale, key, started));
                        STALE_LATER_ANY.store(true, std::sync::atomic::Ordering::Relaxed);
                        BAKE_SLOTS[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    _ => self.unmake(&mut made, stale),
                }
            }
            // A view of a texture object that has no host copy yet waits
            // for it, as buffer views wait for a draw: the slot is made
            // when the texture is first used (`super::make_waiting`), or
            // here when a draw reads the slot through a table.
            let waiting = if for_draw { None } else { view.waiting_texture() };
            if let Some(texture) = waiting {
                super::wait_for(texture, handle);
            }
            if make_now {
                SYNC_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let later = !for_draw && !make_now && (view.is_buffer() || waiting.is_some());
            // Another view of the placed texture the slot shows already (a
            // title that streams a texture in writes its view again with
            // more mips each time some arrive): the slot goes on showing
            // the old view until a draw or the texture pass makes the new
            // one and uploads what it shows. Made null here, the texture
            // read as nothing for as long as it waited in the queue, every
            // time it got sharper.
            if later
                && keep_views()
                && view_at(made[index as usize] as u32, (made[index as usize] >> 32) as u32)
                    .is_some_and(|old| same_placed_texture(&old, &view))
            {
                KEPT_VIEWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Some(view);
            }
            made[index as usize] = if later { 0 } else { key };
            // Cleared while the slot is made: a mirroring without the lock
            // never takes it for made from this record before it is.
            self.made_fast_set(index, 0);
            let kept = if later {
                None
            } else {
                make(&view, self.cpu(index))
            };
            if kept.is_none() && !self.sampler {
                null_for(&view, self.cpu(index));
            }
            let kept = kept.flatten();
            let shows = kept.is_some();
            self.keep(index, kept);
            self.made_fast_set(index, made[index as usize]);
            if let Some(sig) = sig
                && shows
            {
                self.note_shown(index, sig);
            }
            // Where the books would have marked the slot for the pass (it is
            // one of the buffer views, or was): the range the pass would
            // take, given now. Without a copy, marked as before.
            if gives_range && SYNC_NOTED.get() {
                match buffer_copy(&view) {
                    Some((copy, address, len)) => {
                        self.buffers.take_range(index, (copy, address, address + len));
                        SYNC_TAKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    None => self.buffers.written(index, true),
                }
            }
            #[cfg(feature = "diagnostics")]
            if buffer_view_kind(&view) {
                let us = making.elapsed().as_micros() as u64;
                let part = if in_heap_pass() { 0 } else if !later { 2 } else { 4 };
                MIRROR_SPLIT[part].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                MIRROR_SPLIT[part + 1].fetch_add(us, std::sync::atomic::Ordering::Relaxed);
            }
        } else if for_draw && in_heap_pass() {
            SAME_VIEWS[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Some(view)
    }

    /// Whether title slot `index`'s host descriptor was made from a view
    /// with fingerprint `sig` ([`same_view_signature`]) and still shows the
    /// host copy `view` would be made on now: its descriptor would be the
    /// same. A copy let go and made anew, or the memory now in another
    /// copy, gives another copy, and the slot is made again.
    fn shows_same(&self, index: u32, sig: u64, view: &View) -> bool {
        if self.shown.lock().unwrap_or_else(|e| e.into_inner()).get(index as usize) != Some(&sig) {
            return false;
        }
        let Some((copy, _, _)) = buffer_copy(view) else {
            return false;
        };
        self.kept_piece(index).is_some_and(|(kept, at)| {
            kept[at]
                .as_ref()
                .is_some_and(|shown| windows::core::Interface::as_raw(shown) == windows::core::Interface::as_raw(&copy.buffer))
        })
    }

    /// Title slot `index`'s host descriptor was just made from a view with
    /// fingerprint `sig`.
    fn note_shown(&self, index: u32, sig: u64) {
        let mut shown = self.shown.lock().unwrap_or_else(|e| e.into_inner());
        if shown.is_empty() {
            shown.resize(self.count as usize, 0);
        }
        if let Some(slot) = shown.get_mut(index as usize) {
            *slot = sig;
        }
    }

    /// Whether slot `index` is backed by a made host descriptor (tests).
    #[cfg(test)]
    pub(crate) fn made(&self, index: u32) -> bool {
        self.made
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(index as usize)
            .is_some_and(|&key| key != 0)
    }

    /// Whether host slot `index` shows `buffer`; `None` if it shows no
    /// resource (diagnostics: a buffer view's slot must show the host copy
    /// that is brought up to date for it; `crate::material_demand`: the
    /// record read is the one the shader reads).
    pub(crate) fn shows(&self, index: u32, buffer: &ID3D12Resource) -> Option<bool> {
        let (kept, at) = self.kept_piece(index)?;
        let shown = kept[at].as_ref()?;
        Some(windows::core::Interface::as_raw(shown) == windows::core::Interface::as_raw(buffer))
    }

    /// What the queue of textures picked by index knows of title slot
    /// `index` at `frame`, in a few words (diagnostics).
    #[cfg(feature = "diagnostics")]
    pub(crate) fn texture_slot_state(&self, index: u32, frame: u64) -> String {
        let textures = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        match textures.slots.get(&index) {
            None => "not queued".into(),
            Some(w) => format!(
                "queued due={} written@{} looks={} part_mapped={} recheck={}",
                if w.due <= frame { "now".to_string() } else { format!("@{}", w.due) },
                w.written,
                w.looks,
                w.mapped,
                w.relook.is_some()
            ),
        }
    }

    /// The view host slot `index` was made from, in a few words, with the
    /// title and host addresses of a buffer view (diagnostics). "null" for
    /// a slot never made or left null.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn describe_slot(&self, index: u32) -> Option<String> {
        let key = *self.made.lock().unwrap_or_else(|e| e.into_inner()).get(index as usize)?;
        if key == 0 {
            return Some("null".into());
        }
        let view = view_at(key as u32, (key >> 32) as u32)?;
        let mut text: String = format!("{view:?}").chars().take(120).collect();
        if let Some((address, len)) = buffer_range(&view) {
            let host = crate::shadow::find(address, len.max(1)).map(|s| s.gpu(address));
            text.push_str(&format!(" title {address:#x}+{len:#x} host {host:#x?}"));
        }
        Some(text)
    }

    /// What this heap knows of the texture view at title slot `index`, for
    /// the bake probe (`crate::bake_probe`): whether its host slot shows a
    /// host resource, which, and where the slot stands in the queue of
    /// textures picked by index. Reads state only.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn probe_slot(&self, index: u32) -> SlotProbe {
        let host = self
            .kept_piece(index)
            .and_then(|(kept, at)| kept[at].as_ref().map(|r| windows::core::Interface::as_raw(r) as usize))
            .unwrap_or(0);
        let shown = host != 0;
        let queue = self.textures.lock().unwrap_or_else(|e| e.into_inner());
        let w = queue.slots.get(&index).copied();
        SlotProbe {
            shown,
            host,
            queued: w.is_some(),
            due: w.map_or(0, |w| w.due),
            written: w.map_or(0, |w| w.written),
            looks: w.map_or(0, |w| w.looks),
            changes: w.map_or(0, |w| w.changes),
            mapped: w.map_or(0, |w| w.mapped),
            stale: w.is_some_and(|w| w.stale),
            relook: w.and_then(|w| w.relook).map(|r| (r.print, r.uploaded)),
        }
    }
}

/// What [`HostHeap::probe_slot`] tells of one title slot: its host slot
/// shows a host resource (`shown`, `host`: its address), and its entry in
/// the queue of textures picked by index, if it has one: the frame from
/// which it is due, the frame it was written, the looks that found nothing
/// to upload, the looks that found its memory changed, the bytes of its
/// memory that were mapped at its last partial upload, whether a look found
/// it changed and it waits for an upload, and (for a slot uploaded whole)
/// the fingerprint of its memory at the upload and the frame of the upload.
#[cfg(feature = "diagnostics")]
#[derive(Debug, Clone, Copy)]
pub(crate) struct SlotProbe {
    pub(crate) shown: bool,
    pub(crate) host: usize,
    pub(crate) queued: bool,
    pub(crate) due: u64,
    pub(crate) written: u64,
    pub(crate) looks: u8,
    pub(crate) changes: u8,
    pub(crate) mapped: u64,
    pub(crate) stale: bool,
    pub(crate) relook: Option<(u64, u64)>,
}

/// Logs `what` the first time it happens.
macro_rules! once {
    ($($arg:tt)*) => {{
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| tracing::warn!($($arg)*));
    }};
}

/// A view description from its words, with the format as the host has it.
fn srv_desc(words: &[u32; 10]) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
    let mut words = *words;
    words[0] = crate::resource::pc_format(words[0]);
    // SAFETY: 40 bytes in the host's layout.
    unsafe {
        std::ptr::from_ref(&words)
            .cast::<D3D12_SHADER_RESOURCE_VIEW_DESC>()
            .read_unaligned()
    }
}

/// The unsigned integer format with the layout of a 16-bit float or
/// signed integer format (one, two or four channels), if `format` is one.
pub(crate) fn half_format(format: u32) -> Option<u32> {
    match format {
        // R16G16B16A16_FLOAT and _SINT: R16G16B16A16_UINT.
        10 | 14 => Some(12),
        // R16G16_FLOAT and _SINT: R16G16_UINT.
        34 | 38 => Some(36),
        // R16_FLOAT and _SINT: R16_UINT.
        54 | 59 => Some(57),
        _ => None,
    }
}

/// `view` as a shader is shown it where it moves a buffer's 16-bit
/// channels as unsigned integers
/// (`victory_shader::emit::HostShader::half_views`): a buffer view in a
/// 16-bit float or signed format, in the unsigned format of the same
/// layout, through which every 16-bit pattern is read and stored as it
/// is. `None` for every other view: the title's own serves (an unsigned
/// one already carries the bits; a texture can only be viewed in the
/// formats it was made for).
pub(crate) fn half_view(view: &View) -> Option<View> {
    // Buffer views: dimension 1 in the title's description of either kind.
    const BUFFER: u32 = 1;
    match view {
        View::Shader(resource, Some(desc)) if desc[1] == BUFFER => {
            let mut desc = *desc;
            desc[0] = half_format(desc[0])?;
            Some(View::Shader(*resource, Some(desc)))
        }
        View::Unordered(resource, counter, Some(desc)) if desc[1] == BUFFER => {
            let mut desc = *desc;
            desc[0] = half_format(desc[0])?;
            Some(View::Unordered(*resource, *counter, Some(desc)))
        }
        View::Placed(unordered, resource, desc) if resource[0] == BUFFER && desc[1] == BUFFER => {
            let mut desc = *desc;
            desc[0] = half_format(desc[0])?;
            Some(View::Placed(*unordered, *resource, desc))
        }
        _ => None,
    }
}

/// The format of `view`, a buffer view through which a shader moves
/// 16-bit channels as unsigned integers
/// (`victory_shader::emit::HostShader::half_views`), if those integers
/// are not what the console gives: a format of wider channels, a
/// normalised one or a signed 8-bit one, whose values the console converts
/// to and from halves. `None` for the formats the integers serve exactly
/// (16 bits a channel of any kind, through [`half_view`] where needed, and
/// unsigned channels of fewer bits; the title's are 16-bit unsigned) and
/// for any other view.
pub(crate) fn half_inexact(view: &View) -> Option<u32> {
    // Buffer views: dimension 1 in the title's description of either kind.
    const BUFFER: u32 = 1;
    // R16G16B16A16, R16G16 and R16 (float, unsigned, signed), then
    // R10G10B10A2, R8G8B8A8, R8G8 and R8 unsigned.
    const EXACT: [u32; 13] = [10, 12, 14, 34, 36, 38, 54, 57, 59, 25, 30, 50, 62];
    let format = match view {
        View::Shader(_, Some(desc)) | View::Unordered(_, _, Some(desc)) if desc[1] == BUFFER => {
            desc[0]
        }
        View::Placed(_, resource, desc) if resource[0] == BUFFER && desc[1] == BUFFER => desc[0],
        _ => return None,
    };
    (!EXACT.contains(&format)).then_some(format)
}

fn uav_desc(words: &[u32; 10]) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
    let mut words = *words;
    words[0] = crate::resource::pc_format(words[0]);
    // SAFETY: 40 bytes in the host's layout.
    unsafe {
        std::ptr::from_ref(&words)
            .cast::<D3D12_UNORDERED_ACCESS_VIEW_DESC>()
            .read_unaligned()
    }
}

/// A buffer view of title memory at `address` on its host copy: the first
/// element moves by the copy's offset.
/// Bytes per element of a buffer view.
fn element_size(desc: &D3D12_SHADER_RESOURCE_VIEW_DESC) -> u64 {
    // SAFETY: a buffer view's union member.
    let buffer = unsafe { desc.Anonymous.Buffer };
    if buffer.StructureByteStride > 0 {
        u64::from(buffer.StructureByteStride)
    } else if buffer.Flags.0 & D3D12_BUFFER_SRV_FLAG_RAW.0 != 0 {
        4
    } else {
        crate::resource::element(desc.Format.0 as u32).map_or(0, |(bytes, _)| bytes)
    }
}

/// The title address a buffer view or a ray-tracing structure view starts
/// at; 0 for other views.
fn view_address(view: &View) -> u64 {
    if let Some((start, _)) = buffer_range(view) {
        return start;
    }
    match view {
        View::Shader(_, Some(words)) => {
            let desc = srv_desc(words);
            if desc.ViewDimension == D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE {
                // SAFETY: the union member of that dimension.
                unsafe { desc.Anonymous.RaytracingAccelerationStructure.Location }
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// The title memory a buffer shader or unordered access view reads: its
/// address and length.
pub(crate) fn buffer_range(view: &View) -> Option<(u64, u64)> {
    let (base, words): (u64, &[u32; 10]) = match view {
        View::Shader(r, Some(desc)) => (
            crate::resource::buffer_bytes(*r as crate::object::This)?.0 as u64,
            desc,
        ),
        View::Placed(false, resource, desc) if resource[0] == 1 => (
            u64::from(desc[10]) | u64::from(desc[11]) << 32,
            desc[..10].try_into().ok()?,
        ),
        View::Unordered(r, _, Some(desc)) => {
            let base = crate::resource::buffer_bytes(*r as crate::object::This)?.0 as u64;
            return unordered_range(base, desc);
        }
        View::Placed(true, resource, desc) if resource[0] == 1 => {
            let base = u64::from(desc[10]) | u64::from(desc[11]) << 32;
            return unordered_range(base, desc[..10].try_into().ok()?);
        }
        _ => return None,
    };
    let desc = srv_desc(words);
    let element = element_size(&desc);
    // SAFETY: a buffer view's union member.
    let buffer = unsafe { desc.Anonymous.Buffer };
    Some((
        base + buffer.FirstElement * element,
        u64::from(buffer.NumElements) * element,
    ))
}

/// The title memory the host slot of buffer view `view` is made on
/// (address, bytes): the whole buffer the view is of, which is what
/// [`make`] finds the host copy by. [`buffer_range`] is the part of it the
/// view reads.
pub(crate) fn buffer_extent(view: &View) -> Option<(u64, u64)> {
    match view {
        View::Shader(r, Some(_)) | View::Unordered(r, _, Some(_)) => {
            let (data, len) = crate::resource::buffer_bytes(*r as crate::object::This)?;
            Some((data as u64, len))
        }
        View::Placed(_, resource, desc) if resource[0] == 1 => Some((
            u64::from(desc[10]) | u64::from(desc[11]) << 32,
            u64::from(resource[4]) | u64::from(resource[5]) << 32,
        )),
        _ => None,
    }
}

/// Whether a buffer view is brought up to date in the host copy its slot
/// shows ([`buffer_copy`]). On by default; `VICTORY_VIEW_EXTENT=0` looks the
/// copy up by the view's range alone, as before.
fn view_extent() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_VIEW_EXTENT").is_none_or(|v| v != "0"))
}

/// The host copy buffer view `view` reads, and the title range (address,
/// bytes) it reads there; `None` for other views and empty ranges.
///
/// The copy is the one the view's slot shows: [`make`] finds it by the
/// whole buffer the view is of ([`buffer_extent`]). Looked up by the range
/// the view reads, as it was, the copy is another one wherever the two
/// lookups differ: for memory outside buffer objects, a view of a part of a
/// buffer that crosses a window line (the buffer's window is two cells, the
/// part's one), and for buffer objects, a part that also lies in a smaller
/// buffer object. The slot then showed a copy nothing brought up to date,
/// and what shaders wrote through it was looked for in the other copy.
pub(crate) fn buffer_copy(view: &View) -> Option<(std::sync::Arc<crate::shadow::Shadow>, u64, u64)> {
    if let &View::Constant(address, size) = view {
        return heap_constants()
            .then(|| constant_copy(address, size))
            .flatten()
            .map(|(shadow, address, len, _)| (shadow, address, len));
    }
    let (address, len) = buffer_range(view)?;
    if len == 0 {
        return None;
    }
    // A view off its buffer's elements is made on a copy of its own bytes:
    // the same question `buffer_srv` (`off_element`: off its elements, or
    // untyped and off 16 bytes) and `buffer_uav` (`own_bytes`) ask, of the
    // copy they find, so that the copy brought up to date is the one the
    // slot shows.
    if let Some(words) = shader_buffer_words(view) {
        if own_copy_on()
            && let Some((base, width)) = buffer_extent(view)
            && let Some(shadow) = crate::shadow::find(base, width)
            && off_element(shadow.offset(base), &srv_desc(words))
        {
            return crate::shadow::exact(address, len).map(|own| (own, address, len));
        }
    } else if let Some((base, width)) = buffer_extent(view)
        && let Some((first, count, element, stride, unknown)) = buffer_fields(view)
        && let Some(whole) = crate::shadow::find(base, width)
        && let Some((start, bytes)) = own_bytes(whole.offset(base), base, first, count, element, stride, unknown)
        && let Some(own) = crate::shadow::exact(start, bytes)
    {
        return Some((own, start, bytes));
    }
    if view_extent()
        && let Some((base, width)) = buffer_extent(view)
        // (The console is lax: a view may begin past its buffer's end.)
        && address >= base
        && address - base < width.max(1)
        && let Some(shadow) = crate::shadow::find(base, width)
    {
        return Some((shadow, address, len));
    }
    crate::shadow::find(address, len).map(|shadow| (shadow, address, len))
}

/// The description of a shader (not unordered access) view of buffer
/// memory, as the title wrote it.
fn shader_buffer_words(view: &View) -> Option<&[u32; 10]> {
    match view {
        View::Shader(r, Some(desc)) => {
            crate::resource::buffer_bytes(*r as crate::object::This)?;
            Some(desc)
        }
        View::Placed(false, resource, desc) if resource[0] == 1 => desc[..10].try_into().ok(),
        _ => None,
    }
}

/// Whether a shader buffer view `offset` bytes into the host copy of the
/// memory it is in cannot be made on that copy: it is no raw window
/// ([`raw_window`]) and does not begin on one of its elements there (a
/// typed view's first element is counted in its elements, and the console
/// lets a view begin anywhere: 121 `R32G32B32_FLOAT` views of 264-792
/// bytes a run, #62). Such a view is made on a copy of its own range
/// instead ([`crate::shadow::exact`]), where it begins at element 0; it was
/// left null, and shaders read zeros.
///
/// So is a view of untyped bytes that is on an element but not on 16 bytes
/// of the copy, where the host's raw views begin ([`off_raw`]).
fn off_element(offset: u64, desc: &D3D12_SHADER_RESOURCE_VIEW_DESC) -> bool {
    own_copy(offset, desc).is_some()
}

/// Why a shader buffer view needs a copy of its own range
/// ([`off_element`]), as the index of its count in [`VIEW_FATES`].
fn own_copy(offset: u64, desc: &D3D12_SHADER_RESOURCE_VIEW_DESC) -> Option<usize> {
    // SAFETY: a buffer view's union member.
    let b = unsafe { desc.Anonymous.Buffer };
    let window = desc.Format == DXGI_FORMAT_UNKNOWN
        && raw_window(offset, b.FirstElement, b.NumElements, b.StructureByteStride).is_some();
    let element = element_size(desc);
    if window || element == 0 {
        return None;
    }
    if !offset.is_multiple_of(element) {
        return Some(FATE_OWN_TYPED);
    }
    off_raw(offset, desc)
}

/// A view of untyped bytes (a raw view of the title's, or a structured one
/// of whole dwords, which translated shaders read as raw bytes) that
/// begins on an element of its memory's copy but not on 16 bytes of it.
/// The host's raw views begin on 16 bytes and the console's anywhere, so
/// the title's raw view was refused ([`buffer_view_ok`]) and left null,
/// 440 to 2,249 of them a run on a Radeon RX 7800 XT depending on where the
/// title's allocations fell, and the structured one stayed structured,
/// through which raw reads are not the bytes ([`raw_window`]). On a copy of
/// its own range either begins at byte 0 and is a raw view.
fn off_raw(offset: u64, desc: &D3D12_SHADER_RESOURCE_VIEW_DESC) -> Option<usize> {
    // SAFETY: a buffer view's union member.
    let b = unsafe { desc.Anonymous.Buffer };
    let element = element_size(desc);
    let start = b.FirstElement.checked_mul(element)?.checked_add(offset)?;
    if start.is_multiple_of(16) || b.NumElements == 0 {
        return None;
    }
    let raw = b.Flags.0 & D3D12_BUFFER_SRV_FLAG_RAW.0 != 0 && b.StructureByteStride == 0;
    let structured = desc.Format == DXGI_FORMAT_UNKNOWN
        && b.StructureByteStride > 0
        && (u64::from(b.NumElements) * element).is_multiple_of(4);
    if raw && desc.Format == DXGI_FORMAT_R32_TYPELESS {
        Some(FATE_OWN_RAW)
    } else if structured {
        Some(FATE_OWN_STRUCTURED)
    } else {
        None
    }
}

/// [`buffer_range`] of an unordered access view of memory at `base`.
fn unordered_range(base: u64, words: &[u32; 10]) -> Option<(u64, u64)> {
    let desc = uav_desc(words);
    // SAFETY: a buffer view's union member.
    let buffer = unsafe { desc.Anonymous.Buffer };
    let element = if buffer.StructureByteStride > 0 {
        u64::from(buffer.StructureByteStride)
    } else if buffer.Flags.0 & D3D12_BUFFER_UAV_FLAG_RAW.0 != 0 {
        4
    } else {
        crate::resource::element(desc.Format.0 as u32)?.0
    };
    Some((
        base + buffer.FirstElement * element,
        u64::from(buffer.NumElements) * element,
    ))
}

/// The host resource a slot made by [`make`] views, if any.
type Kept = Option<ID3D12Resource>;

/// Whether the pass over records copied in as bytes also mirrors slots
/// that draws read through tables lately (`VICTORY_HEAP_SYNC_TABLES=1`;
/// off by default: [`HostHeap::mirror_copied`]).
fn sync_tables() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_HEAP_SYNC_TABLES").is_some_and(|v| v == "1"))
}

/// Since the last frame line: slots on changed heap pages that
/// [`HostHeap::mirror_copied`] left to their draws (tables' slots).
pub(crate) static COPIED_TABLED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A 64-bit fingerprint of a descriptor record's bytes (16 or 32 of them):
/// equal records give equal prints, and two different records the same
/// print about once in 2^64. Not a hash anyone can attack: the records
/// are our own.
fn record_print(record: &[u8]) -> u64 {
    const K: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut h = K ^ record.len() as u64;
    let mut words = record.chunks_exact(8);
    for word in &mut words {
        let w = u64::from_le_bytes(word.try_into().unwrap_or([0; 8]));
        h = (h ^ w).wrapping_mul(K);
        h ^= h >> 29;
    }
    for &b in words.remainder() {
        h = (h ^ u64::from(b)).wrapping_mul(K);
    }
    h ^ (h >> 32)
}

/// Whether a buffer view off its buffer's elements gets a copy of its own
/// bytes (`VICTORY_VIEW_OWN_COPY=0`: it is refused and reads zeros, as
/// before).
fn own_copy_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_VIEW_OWN_COPY").is_none_or(|v| v != "0"))
}

/// The title memory (start, bytes) of a buffer view that cannot be made on
/// the host copy of its buffer: `count` elements of `element` bytes from
/// element `first` of the buffer at `base`, which lies `offset` bytes into
/// that copy, where `offset` is not a whole number of elements and the view
/// cannot be a raw view of the same bytes either ([`raw_window`]:
/// `unknown`, the view has no format, and `stride` is its structure's).
/// The host counts a view's first element from its buffer's start, so such
/// a view has no host view there; it is made on a copy of its own bytes
/// (`crate::shadow::exact`), where it starts at element 0. `None`: the
/// view is made on its buffer's copy, as every other.
fn own_bytes(offset: u64, base: u64, first: u64, count: u32, element: u64, stride: u32, unknown: bool) -> Option<(u64, u64)> {
    own_bytes_if(own_copy_on(), offset, base, first, count, element, stride, unknown)
}

#[allow(clippy::too_many_arguments)]
fn own_bytes_if(
    on: bool,
    offset: u64,
    base: u64,
    first: u64,
    count: u32,
    element: u64,
    stride: u32,
    unknown: bool,
) -> Option<(u64, u64)> {
    if !on || element == 0 || count == 0 || offset.is_multiple_of(element) {
        return None;
    }
    if unknown && raw_window(offset, first, count, stride).is_some() {
        return None;
    }
    let start = base.checked_add(first.checked_mul(element)?)?;
    Some((start, u64::from(count) * element))
}

/// Of a buffer view: its first element, its elements, the bytes of one (a
/// raw view's: 4), its structure's stride, and whether it has no format.
/// As [`buffer_srv`] and [`buffer_uav`] read them from the same words.
fn buffer_fields(view: &View) -> Option<(u64, u32, u64, u32, bool)> {
    const BUFFER: u32 = 1;
    let (words, unordered): (&[u32; 10], bool) = match view {
        View::Shader(_, Some(d)) if d[1] == BUFFER => (d, false),
        View::Unordered(_, _, Some(d)) if d[1] == BUFFER => (d, true),
        View::Placed(unordered, resource, d) if resource[0] == BUFFER => (d[..10].try_into().ok()?, *unordered),
        _ => return None,
    };
    if unordered {
        let d = uav_desc(words);
        // SAFETY: a buffer view's union member.
        let b = unsafe { d.Anonymous.Buffer };
        let element = if b.StructureByteStride > 0 {
            u64::from(b.StructureByteStride)
        } else if b.Flags.0 & D3D12_BUFFER_UAV_FLAG_RAW.0 != 0 {
            4
        } else {
            crate::resource::element(d.Format.0 as u32).map_or(0, |(bytes, _)| bytes)
        };
        Some((b.FirstElement, b.NumElements, element, b.StructureByteStride, d.Format == DXGI_FORMAT_UNKNOWN))
    } else {
        let d = srv_desc(words);
        // SAFETY: a buffer view's union member.
        let b = unsafe { d.Anonymous.Buffer };
        Some((b.FirstElement, b.NumElements, element_size(&d), b.StructureByteStride, d.Format == DXGI_FORMAT_UNKNOWN))
    }
}

/// A structured buffer view (`first`, `count` elements of `stride` bytes,
/// `offset` bytes into its host copy) as a raw view of the same bytes:
/// its first dword and its dwords. Translated shaders read untyped buffers
/// as raw bytes at `element * stride + offset` (the stride from the
/// used-resource entry, or from the table by heap slot for a buffer picked
/// by index), and a raw read through a structured view gives zeros on
/// NVIDIA (reported upstream with PR #193; structured reads through raw
/// views were right there). So the host's view is raw whenever it can be:
/// the bytes must start on 16 (the host's raw view alignment) and be whole
/// dwords. A view that cannot stays structured.
fn raw_window(offset: u64, first: u64, count: u32, stride: u32) -> Option<(u64, u32)> {
    if stride == 0 {
        return None;
    }
    let start = first.checked_mul(u64::from(stride))?.checked_add(offset)?;
    let bytes = u64::from(count) * u64::from(stride);
    (start % 16 == 0 && bytes % 4 == 0)
        .then(|| u32::try_from(bytes / 4).ok().map(|dwords| (start / 4, dwords)))
        .flatten()
}

/// Whether the host accepts a buffer view with these fields of a buffer of
/// `width` bytes. It removes the device for one it does not (view creation
/// returns nothing), and the console is laxer: the title's heap holds
/// views past their memory's end, of no elements, of typeless formats
/// without the raw flag, with strides the host has no structured buffers
/// for. Since every buffer view of the heap is made for shaders that pick
/// them by index, one such view ended the run (2026-11-24, run 10).
fn buffer_view_ok(format: DXGI_FORMAT, first: u64, count: u32, stride: u32, raw: bool, width: u64) -> bool {
    use crate::resource::family;
    let f = format.0 as u32;
    let element = if raw {
        // raw: R32 typeless, counted in dwords, starting on 16 bytes
        if format != DXGI_FORMAT_R32_TYPELESS || stride != 0 || !first.is_multiple_of(4) {
            return false;
        }
        4
    } else if stride > 0 {
        // structured: no format, a stride the host takes
        if format != DXGI_FORMAT_UNKNOWN || stride > 2048 || !stride.is_multiple_of(4) {
            return false;
        }
        u64::from(stride)
    } else {
        // typed: a format with a size that is not typeless
        let typeless = family(f) == f && family(f + 1) == f;
        match crate::resource::element(f) {
            Some((bytes, 1)) if f != 0 && !typeless && bytes > 0 => bytes,
            _ => return false,
        }
    };
    count > 0
        && first
            .checked_add(u64::from(count))
            .and_then(|end| end.checked_mul(element))
            .is_some_and(|end| end <= width)
}

fn buffer_srv(
    address: u64,
    len: u64,
    mut desc: D3D12_SHADER_RESOURCE_VIEW_DESC,
    at: D3D12_CPU_DESCRIPTOR_HANDLE,
) -> Option<Kept> {
    let mut shadow = crate::shadow::find(address, len)?;
    let mut offset = shadow.offset(address);
    // Off the copy's elements: on a copy of the view's own range, from its
    // element 0 (see `off_element`; `buffer_copy` names the same copy).
    // `VICTORY_VIEW_OWN_COPY=0`: as the host would make it, or refused.
    if own_copy_on()
        && let Some(why) = own_copy(offset, &desc)
    {
        fate(why);
        let element = element_size(&desc);
        // SAFETY: a buffer view's union member.
        let buffer = unsafe { &mut desc.Anonymous.Buffer };
        let start = buffer.FirstElement.checked_mul(element)?.checked_add(address)?;
        shadow = crate::shadow::exact(start, u64::from(buffer.NumElements) * element)?;
        buffer.FirstElement = 0;
        offset = 0;
    }
    // A structured view as a raw view of the same bytes (see `raw_window`).
    // SAFETY: a buffer view's union member.
    let structured = unsafe { desc.Anonymous.Buffer };
    let window = (desc.Format == DXGI_FORMAT_UNKNOWN)
        .then(|| raw_window(offset, structured.FirstElement, structured.NumElements, structured.StructureByteStride))
        .flatten();
    if let Some((first, count)) = window {
        desc.Format = DXGI_FORMAT_R32_TYPELESS;
        desc.Anonymous.Buffer = D3D12_BUFFER_SRV {
            FirstElement: first,
            NumElements: count,
            StructureByteStride: 0,
            Flags: D3D12_BUFFER_SRV_FLAG_RAW,
        };
    }
    let element = element_size(&desc);
    // SAFETY: a buffer view's union member.
    let buffer = unsafe { &mut desc.Anonymous.Buffer };
    if window.is_none() {
        if element == 0 || offset % element != 0 {
            fate(FATE_OFF_ELEMENT);
            once!(
                offset,
                element,
                "buffer view not on an element of its host copy"
            );
            return None;
        }
        buffer.FirstElement += offset / element;
    }
    let (first, count, stride) = (buffer.FirstElement, buffer.NumElements, buffer.StructureByteStride);
    let raw = buffer.Flags.0 & D3D12_BUFFER_SRV_FLAG_RAW.0 != 0;
    // SAFETY: a live buffer.
    let width = unsafe { shadow.buffer.GetDesc() }.Width;
    if !buffer_view_ok(desc.Format, first, count, stride, raw, width) {
        REFUSED_VIEWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if count == 0 {
            fate(FATE_NO_ELEMENTS);
        } else if raw && !first.is_multiple_of(4) {
            fate(FATE_RAW_OFF_16);
        }
        once!(
            format = desc.Format.0,
            first,
            count,
            stride,
            raw,
            width,
            "buffer view the host would refuse left null"
        );
        return None;
    }
    // The console has no component mapping; one the host does not know is the default's.
    if desc.Shader4ComponentMapping != D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING {
        desc.Shader4ComponentMapping = D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
    }
    let host = host()?;
    // SAFETY: a live buffer and a slot of our heap.
    unsafe {
        host.device
            .CreateShaderResourceView(&shadow.buffer, Some(&desc), at)
    };
    Some(Some(shadow.buffer.clone()))
}

/// An unordered access view of title buffer memory at `address` (`len`
/// bytes) on its host copy, as [`buffer_srv`]; false if the copy cannot be
/// written by shaders (upload memory).
/// shortcut: what shaders write stays in the host copy; title memory (and
/// so the CPU, and copies run on the CPU) does not see it.
fn buffer_uav(
    address: u64,
    len: u64,
    mut desc: D3D12_UNORDERED_ACCESS_VIEW_DESC,
    at: D3D12_CPU_DESCRIPTOR_HANDLE,
) -> Option<Kept> {
    let shadow = crate::shadow::find(address, len)?;
    // Off the elements of that copy: on a copy of its own bytes, from its
    // element 0 (`own_bytes`).
    let (shadow, own) = {
        // SAFETY: a buffer view's union member.
        let b = unsafe { desc.Anonymous.Buffer };
        let element = if b.StructureByteStride > 0 {
            u64::from(b.StructureByteStride)
        } else if b.Flags.0 & D3D12_BUFFER_UAV_FLAG_RAW.0 != 0 {
            4
        } else {
            crate::resource::element(desc.Format.0 as u32).map_or(0, |(bytes, _)| bytes)
        };
        let unknown = desc.Format == DXGI_FORMAT_UNKNOWN;
        match own_bytes(shadow.offset(address), address, b.FirstElement, b.NumElements, element, b.StructureByteStride, unknown)
            .and_then(|(start, bytes)| crate::shadow::exact(start, bytes))
        {
            Some(own) => {
                desc.Anonymous.Buffer.FirstElement = 0;
                (own, true)
            }
            None => (shadow, false),
        }
    };
    // SAFETY: a live buffer.
    let flags = unsafe { shadow.buffer.GetDesc() }.Flags;
    if flags.0 & D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS.0 == 0 {
        once!("unordered access to a buffer whose host copy shaders cannot write");
        return None;
    }
    // SAFETY: a buffer view's union member.
    let buffer = unsafe { &mut desc.Anonymous.Buffer };
    let element = if buffer.StructureByteStride > 0 {
        u64::from(buffer.StructureByteStride)
    } else if buffer.Flags.0 & D3D12_BUFFER_UAV_FLAG_RAW.0 != 0 {
        4
    } else {
        crate::resource::element(desc.Format.0 as u32).map_or(0, |(bytes, _)| bytes)
    };
    let offset = if own { 0 } else { shadow.offset(address) };
    // A structured view as a raw view of the same bytes (see `raw_window`).
    let window = (desc.Format == DXGI_FORMAT_UNKNOWN)
        .then(|| raw_window(offset, buffer.FirstElement, buffer.NumElements, buffer.StructureByteStride))
        .flatten();
    if let Some((first, count)) = window {
        buffer.FirstElement = first;
        buffer.NumElements = count;
        buffer.StructureByteStride = 0;
        buffer.Flags = D3D12_BUFFER_UAV_FLAG_RAW;
    } else {
        if element == 0 || offset % element != 0 {
            fate(FATE_UNORDERED_OFF_ELEMENT);
            once!(offset, element, "unordered buffer view not on an element of its host copy");
            return None;
        }
        if buffer.StructureByteStride > 0 {
            fate(FATE_UNORDERED_STRUCTURED);
        }
        buffer.FirstElement += offset / element;
    }
    buffer.CounterOffsetInBytes = 0;
    let (first, count, stride) = (buffer.FirstElement, buffer.NumElements, buffer.StructureByteStride);
    let raw = buffer.Flags.0 & D3D12_BUFFER_UAV_FLAG_RAW.0 != 0;
    if window.is_some() {
        // (Left out at first: every structured unordered access view was then
        // refused by `buffer_view_ok` and left null, runs 15 and 16.)
        desc.Format = DXGI_FORMAT_R32_TYPELESS;
    }
    // SAFETY: a live buffer.
    let width = unsafe { shadow.buffer.GetDesc() }.Width;
    if !buffer_view_ok(desc.Format, first, count, stride, raw, width) {
        REFUSED_VIEWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if count == 0 {
            fate(FATE_NO_ELEMENTS);
        } else if raw && !first.is_multiple_of(4) {
            fate(FATE_RAW_OFF_16);
        }
        once!(
            format = desc.Format.0,
            first,
            count,
            stride,
            raw,
            width,
            "unordered buffer view the host would refuse left null"
        );
        return None;
    }
    let host = host()?;
    // SAFETY: a live buffer and a slot of our heap.
    unsafe {
        host.device
            .CreateUnorderedAccessView(&shadow.buffer, None::<&ID3D12Resource>, Some(&desc), at)
    };
    // The view's first byte in title memory, and a typed view's element
    // size: its writes' marks (`crate::marks`), for the slot this goes to.
    let bytes = if raw { 4 } else if stride > 0 { u64::from(stride) } else { element };
    let first_byte = shadow.span().0 + first * bytes;
    let typed = if raw || stride > 0 { 0 } else { element };
    if let Some(entry) = shadow.mark_entry(first_byte, typed) {
        MARK.with(|m| m.set(Some((windows::core::Interface::as_raw(&shadow.buffer) as usize, entry))));
    }
    Some(Some(shadow.buffer.clone()))
}

thread_local! {
    /// The unordered buffer view [`buffer_uav`] last made on this thread,
    /// by its buffer, and its mark table entry: the slot it goes to takes
    /// the entry ([`HostHeap::set_mark`]).
    static MARK: std::cell::Cell<Option<(usize, (u32, u32))>> = const { std::cell::Cell::new(None) };
}

/// Whether the host can make a shader (or unordered access) view of
/// `texture` in `format` (none: the texture's own). The console allows any
/// view; the host refuses (and removes the device for) views without the
/// resource's permission, of typed depth, and in another format than a
/// typed resource's. A view of a typed texture in another format of its
/// family takes the texture's format.
/// shortcut: such a view (sRGB of UNORM, say) reads without its
/// conversion; cast with castable formats if colors come out wrong.
fn fits(texture: &ID3D12Resource, mut format: Option<&mut DXGI_FORMAT>, unordered: bool) -> bool {
    use crate::resource::{DEPTH_FORMATS, family};
    // SAFETY: a live resource.
    let d = unsafe { texture.GetDesc() };
    let allowed = if unordered {
        d.Flags.0 & D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS.0 != 0
    } else {
        d.Flags.0 & D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE.0 == 0
    };
    let own = d.Format.0 as u32;
    if !allowed || DEPTH_FORMATS.iter().any(|&(typed, _)| typed == own) {
        if unordered {
            crate::resource::DEPTH_UNORDERED[3].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        once!(
            unordered,
            format = own,
            flags = d.Flags.0,
            "view the host does not allow"
        );
        return false;
    }
    // Every family has more than one format, its typeless one first.
    let typeless = family(own) == own && family(own + 1) == own;
    if let Some(format) = format.as_deref_mut()
        && unordered
        && let Some(&(_, linear)) = crate::resource::SRGB
            .iter()
            .find(|&&(s, _)| s == format.0 as u32)
    {
        format.0 = linear as i32;
    }
    let view = format.as_ref().map_or(0, |f| f.0 as u32);
    if view == 0 {
        return !typeless;
    }
    // A view of a target whose format the host refuses, in the title's
    // format: in the stand-in the host copy has.
    if view != own && crate::resource::target_format(view) == own {
        if let Some(format) = format {
            *format = d.Format;
        }
        return true;
    }
    if family(view) != family(own) {
        once!(view, own, "view format of another family");
        return false;
    }
    if let Some(format) = format
        && !typeless
    {
        *format = d.Format;
    }
    true
}

/// The title's unordered access view of a depth texture, as a view of the
/// texture's colour twin `twin` ([`crate::resource::Twin`]): the same mip
/// and slices, in the colour format of the twin's family for a depth or
/// typeless format (R16_UNORM for 16-bit depth, R32_FLOAT for 32-bit; the
/// bits are the same). Without a description: mip 0 of every slice.
/// The twin of a stencil plane (R8) stands for the views of that plane
/// only, which become R8_UINT views of its one plane; `None` for a view of
/// the depth plane, which the twin does not stand for. The depth plane has
/// a twin of its own, viewed as a depth texture's without stencil: a view
/// in the format of the depth plane of a depth-stencil format (21, 46)
/// takes the twin's colour format too.
fn twin_uav(
    twin: &ID3D12Resource,
    desc: Option<&D3D12_UNORDERED_ACCESS_VIEW_DESC>,
) -> Option<D3D12_UNORDERED_ACCESS_VIEW_DESC> {
    // SAFETY: a live resource.
    let d = unsafe { twin.GetDesc() };
    let mut desc = desc.cloned().unwrap_or_else(|| {
        let mut all = D3D12_UNORDERED_ACCESS_VIEW_DESC {
            ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        if d.DepthOrArraySize > 1 {
            all.ViewDimension = D3D12_UAV_DIMENSION_TEXTURE2DARRAY;
            all.Anonymous.Texture2DArray = D3D12_TEX2D_ARRAY_UAV {
                ArraySize: u32::from(d.DepthOrArraySize),
                ..Default::default()
            };
        }
        all
    });
    // A stencil plane's twin (R8 typeless, 60): the title's view is of the
    // stencil plane if it names plane 1 or a stencil view format (22, 47).
    if d.Format.0 == 60 {
        let mut plane = 0;
        match desc.ViewDimension {
            D3D12_UAV_DIMENSION_TEXTURE2D => {
                // SAFETY: the member the dimension names.
                let mut view = unsafe { desc.Anonymous.Texture2D };
                plane = std::mem::take(&mut view.PlaneSlice);
                desc.Anonymous.Texture2D = view;
            }
            D3D12_UAV_DIMENSION_TEXTURE2DARRAY => {
                // SAFETY: the member the dimension names.
                let mut view = unsafe { desc.Anonymous.Texture2DArray };
                plane = std::mem::take(&mut view.PlaneSlice);
                desc.Anonymous.Texture2DArray = view;
            }
            _ => {}
        }
        if plane != 1 && !matches!(desc.Format.0, 22 | 47) {
            return None;
        }
        desc.Format.0 = 62;
        return Some(desc);
    }
    // None, typeless and depth formats (R16: 0, 53, 55; R32: 0, 39, 40),
    // and those of a depth-stencil format's depth plane (19 to 21, 44 to
    // 46), for the twin of such a plane.
    desc.Format.0 = match (desc.Format.0, d.Format.0) {
        (0 | 53 | 55 | 19..=21 | 44..=46, 53) => 56,
        (0 | 39 | 40 | 19..=21 | 44..=46, 39) => 41,
        (other, _) => other,
    };
    Some(desc)
}

/// The bytes of the host's constant buffer view for a title's of `size`:
/// whole 256s, and at most the 64 KiB a shader can index. The host removes
/// the device for a larger view, and the title's heap holds such sizes in
/// slots it does not use.
/// A shader view `desc` of a texture whose host copy leaves out its
/// `dropped` largest mips (`crate::resource::drop_levels`), made a view of
/// that host copy: its mips moved down by them.
/// (After victory-fork PR #610.)
fn shift_view_levels(dropped: u32, desc: &mut D3D12_SHADER_RESOURCE_VIEW_DESC) {
    if dropped == 0 {
        return;
    }
    let shift = crate::resource::shift_levels;
    // SAFETY: the union member the view's dimension names.
    unsafe {
        match desc.ViewDimension {
            D3D12_SRV_DIMENSION_TEXTURE2D => {
                let t = &mut desc.Anonymous.Texture2D;
                shift(dropped, &mut t.MostDetailedMip, &mut t.MipLevels, &mut t.ResourceMinLODClamp);
            }
            D3D12_SRV_DIMENSION_TEXTURE2DARRAY => {
                let t = &mut desc.Anonymous.Texture2DArray;
                shift(dropped, &mut t.MostDetailedMip, &mut t.MipLevels, &mut t.ResourceMinLODClamp);
            }
            D3D12_SRV_DIMENSION_TEXTURECUBE => {
                let t = &mut desc.Anonymous.TextureCube;
                shift(dropped, &mut t.MostDetailedMip, &mut t.MipLevels, &mut t.ResourceMinLODClamp);
            }
            D3D12_SRV_DIMENSION_TEXTURECUBEARRAY => {
                let t = &mut desc.Anonymous.TextureCubeArray;
                shift(dropped, &mut t.MostDetailedMip, &mut t.MipLevels, &mut t.ResourceMinLODClamp);
            }
            _ => {}
        }
    }
}

fn constant_bytes(size: u32) -> u32 {
    const LARGEST: u32 = 64 << 10;
    size.min(LARGEST).next_multiple_of(256)
}

thread_local! {
    /// Whether this thread is in the pass over the heap's buffer views
    /// (`crate::draw`, [`HeapPass`]).
    static IN_HEAP_PASS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Whether the slot whose books are kept now is made at the mirroring
    /// of copied records and gives its range to the pass itself
    /// ([`sync_ranges`]): noted, not marked changed.
    static SYNC_NOTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Whether those books found the slot one of the buffer views (or a
    /// former one): where marking it would have sent the pass to it.
    static SYNC_NOTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a buffer view made at the mirroring of copied records
/// ([`sync_makes`]) gives its range to the next pass over the heap's buffer
/// views at once, so the pass does not look at the slot
/// (`VICTORY_SYNC_RANGES=0`: it is marked changed and looked at).
///
/// Looked at, each such slot cost the pass about 1.3 us (its record read
/// again, the view found, the copy it shows found): some 35,000 a frame in
/// the open world, 75 to 85 ms of the pass.
fn sync_ranges() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SYNC_RANGES").is_none_or(|v| v != "0"))
}

/// Since the last frame line: ranges given to the pass by slots made at
/// the mirroring ([`sync_ranges`]).
pub(crate) static SYNC_TAKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// While it lives, this thread is in the pass over the heap's buffer views.
pub(crate) struct HeapPass(());

impl HeapPass {
    pub(crate) fn enter() -> Self {
        IN_HEAP_PASS.set(true);
        HeapPass(())
    }
}

impl Drop for HeapPass {
    fn drop(&mut self) {
        IN_HEAP_PASS.set(false);
    }
}

fn in_heap_pass() -> bool {
    IN_HEAP_PASS.get()
}

/// Whether the pass over the heap's buffer views looks at a sixteenth of all
/// buffer view slots at each pass (`VICTORY_ROLLING_RESCAN=0`: all of them
/// at every sixteenth pass, as before). All of them at once was some
/// 130,000 slots in the open world, the frames whose pass took 200 to 275
/// ms.
fn rolling_rescan() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_ROLLING_RESCAN").is_none_or(|v| v != "0"))
}

/// Whether a slot the pass over buffer views makes is marked changed for
/// the next pass too (`VICTORY_PASS_REVISITS=1`; off by default).
fn pass_revisits() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_PASS_REVISITS").is_some_and(|v| v == "1"))
}

/// Whether `view` views buffer memory: a constant buffer view, a view of a
/// buffer resource, or a view of memory described as a buffer (not a
/// texture placed in memory).
fn buffer_view_kind(view: &View) -> bool {
    const BUFFER: u32 = 1;
    match view {
        View::Constant(..) => true,
        View::Placed(_, resource, _) => resource[0] == BUFFER,
        View::Shader(r, _) | View::Unordered(r, _, _) => crate::resource::buffer_bytes(*r as crate::object::This).is_some(),
        View::Sampler(_) | View::ConsoleSampler(_) => false,
    }
}

/// Whether the mirroring of records the title copied into its heap as
/// bytes (`super::sync_heaps`) makes the buffer views among them at once,
/// as a draw would (`VICTORY_SYNC_MAKES=0`: made null and left to the pass
/// over the heap's buffer views, as before).
///
/// Such a view was made twice: null when its record was mirrored (with the
/// slot's books: the texture queue, the placed views, the stride table),
/// then again by the next pass over buffer views, which a frame runs once
/// (some 60,000 slots a frame in the open world: 110 ms of the pass). Lists
/// recorded between the two read the null view. Made at once, the pass
/// finds the slot made and only takes its range.
fn sync_makes() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SYNC_MAKES").is_none_or(|v| v != "0"))
}

/// Since the last frame line: buffer views made at the mirroring of copied
/// records ([`sync_makes`]).
pub(crate) static SYNC_MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Since the last frame line (diagnostics), of buffer views a record
/// changed: made by the pass over buffer views, made at once elsewhere (at
/// the mirroring of copied records, or for a draw), and made null for
/// later; each a count and its microseconds (the slot's books, the view
/// made or nulled, the resource kept).
#[cfg(feature = "diagnostics")]
pub(crate) static MIRROR_SPLIT: [std::sync::atomic::AtomicU64; 6] = [const { std::sync::atomic::AtomicU64::new(0) }; 6];

/// Whether a record that names a buffer view the slot already shows leaves
/// the slot as it is ([`HostHeap::mirror`]; `VICTORY_SAME_VIEWS=0`: every
/// new record is made again, as before).
///
/// The title writes its per-frame view records again each frame, under new
/// names but with the same contents, and copies them into the heap as
/// bytes: the mirroring of copied records (`super::sync_heaps`) found
/// 40,000 a frame in the open world, each made null, then made again by the
/// pass over the heap's buffer views (75,000 to 140,000 slots a frame, 105
/// to 150 ms of it making host views).
fn same_views() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SAME_VIEWS").is_none_or(|v| v != "0"))
}

/// Since the last frame line: records of buffer views that named the view
/// their slot showed (left as they were), the other buffer view records
/// (made again), and slots the pass over buffer views looked at that were
/// made already.
pub(crate) static SAME_VIEWS: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

/// The slots of one piece of [`HostHeap::kept`].
const KEPT_PIECE: u32 = 1024;

/// Whether [`HostHeap::mirror`] finds a slot made from the record it
/// holds without taking `made`'s lock ([`HostHeap::made_fast`];
/// `VICTORY_HEAP_LOCKS=0`: the lock is taken every time, as before).
/// Some 50,000 mirrorings a frame, nearly all of them of slots made
/// already, each took the lock (after victory-fork's pull requests #1117,
/// which measured some 50,000 lock takings a frame, and #1146, whose
/// switch this is: with the pieces of `kept`, a fifth of the recording
/// threads' time in list calls on a six-thread processor was waiting for
/// the two locks).
fn mirror_fast() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_HEAP_LOCKS").is_none_or(|v| v != "0"))
}

/// Since the last frame line: mirrorings that found their slot made from
/// the record it holds without taking `made`'s lock ([`mirror_fast`]).
pub(crate) static MIRROR_FAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A fingerprint of a buffer view (constant buffer views, views of buffer
/// resources and of buffer memory) and the title memory it reads, never 0;
/// `None` for other views, and with [`same_views`] off.
fn same_view_signature(view: &View) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    if !same_views() || !buffer_view_kind(view) {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    view.hash(&mut hasher);
    buffer_range(view).hash(&mut hasher);
    Some(hasher.finish() | 1)
}

/// Whether constant buffer views of the view heap (the ones a shader picks
/// by an index it computes) are brought up to date for shaders, and made on
/// a copy of their own range where the copy of the memory they are in does
/// not put them on 256 bytes. On by default; `VICTORY_HEAP_CONSTANTS=0`
/// leaves them as before: made once on whatever copy holds them, never
/// brought up to date by the heap's pass, and left null off 256 bytes.
fn heap_constants() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_HEAP_CONSTANTS").is_none_or(|v| v != "0"))
}

/// The host copy a constant buffer view of `size` bytes at `address` is
/// made on, and the title range (address, bytes) it reads there.
///
/// The console takes a constant buffer anywhere on 16 bytes; the host's
/// begin on 256 bytes of their resource. Where the copy of the memory the
/// view is in does not put it on 256 bytes, the view gets a copy of its own
/// range ([`crate::shadow::exact`]), which begins at its resource's first
/// byte. Such views were left null, and shaders read zeros as constants:
/// some 100,000 views a run (the heap's pass names the same copy, through
/// [`buffer_copy`], and brings it up to date). The copy reaches to the
/// bytes the host's view reads, rounded up to 256.
/// (After Belobug/VIctory-Belobug-AMD 00fc4a7, which copies such views into
/// aligned slices at each submission.)
/// The last of the four says whether the copy is the view's own.
fn constant_copy(address: u64, size: u32) -> Option<(std::sync::Arc<crate::shadow::Shadow>, u64, u64, bool)> {
    let len = u64::from(size);
    let shadow = crate::shadow::find(address, len)?;
    if shadow.gpu(address).is_multiple_of(256) || !heap_constants() {
        return Some((shadow, address, len, false));
    }
    let reach = u64::from(constant_bytes(size));
    let own = crate::shadow::exact(address, reach)?;
    Some((own, address, reach, true))
}

/// Makes the host descriptor for `view` at `at`: the host resource it
/// views, or `None` if it cannot.
fn make(view: &View, at: D3D12_CPU_DESCRIPTOR_HANDLE) -> Option<Kept> {
    // A mark is only for the view made now.
    MARK.with(|m| m.set(None));
    let host = host()?;
    let device = &host.device;
    match view {
        &View::Constant(address, size) => {
            let (shadow, _, _, own) = constant_copy(address, size)?;
            if own {
                fate(FATE_CONSTANT_OWN);
            }
            let gpu = shadow.gpu(address);
            if gpu % 256 != 0 {
                fate(FATE_CONSTANT_OFF_256);
                once!(
                    address = format_args!("{address:#x}"),
                    "constant buffer not 256-byte aligned on the host"
                );
                return None;
            }
            let desc = D3D12_CONSTANT_BUFFER_VIEW_DESC {
                BufferLocation: gpu,
                SizeInBytes: constant_bytes(size),
            };
            // SAFETY: a slot of our heap; the range lies in the host copy.
            unsafe { device.CreateConstantBufferView(Some(&desc), at) };
            Some(Some(shadow.buffer.clone()))
        }
        View::Shader(resource, desc) => {
            let resource = *resource as crate::object::This;
            if let Some(texture) = crate::resource::host_resource(resource) {
                // The title's views of textures with a stencil plane, as it
                // describes them (format, dimension, component mapping, a
                // pad, then first level, levels, plane): each distinct one
                // once. They say how its shaders read the stencil plane
                // (the lighting takes each pixel's material class and its
                // "inside a room" bit from it).
                #[cfg(feature = "diagnostics")]
                {
                    // SAFETY: a live resource.
                    let own = unsafe { texture.GetDesc() }.Format.0;
                    if matches!(own, 19..=22 | 44..=47) {
                        static SEEN: Mutex<Vec<(i32, Option<[u32; 10]>)>> = Mutex::new(Vec::new());
                        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
                        if seen.len() < 64 && !seen.contains(&(own, *desc)) {
                            seen.push((own, *desc));
                            tracing::info!(
                                ?resource,
                                texture_format = own,
                                view = ?desc,
                                "shader view of a depth-stencil texture"
                            );
                        }
                    }
                }
                let mut desc = desc.as_ref().map(srv_desc);
                if !fits(texture, desc.as_mut().map(|d| &mut d.Format), false) {
                    return None;
                }
                // Levels on demand (`crate::tiles`): a texture made level by
                // level is shown only through a run of levels that are the
                // title's and have memory, one after the other (a level
                // without memory reads zeros: a black texture; a finer level
                // and the packed tail can arrive before one between them,
                // and a view from the first level alone reached that hole).
                // A texture whose every level has memory keeps the title's
                // view as it is (with none, the default view).
                let resident = crate::resource::shown_levels(resource);
                // SAFETY: a live resource.
                let own = (resident != 0).then(|| unsafe { texture.GetDesc() });
                if let Some(own) = own.filter(|own| {
                    let every = u32::MAX >> (32 - u32::from(own.MipLevels).clamp(1, 32));
                    desc.is_some() || resident & every != every
                }) {
                    // The texture's own view (the title gave none): all of it.
                    let whole = D3D12_SHADER_RESOURCE_VIEW_DESC {
                        Format: own.Format,
                        ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                        Shader4ComponentMapping: 5768,
                        Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                            Texture2D: D3D12_TEX2D_SRV {
                                MostDetailedMip: 0,
                                MipLevels: u32::MAX,
                                PlaneSlice: 0,
                                ResourceMinLODClamp: 0.0,
                            },
                        },
                    };
                    crate::tiles::show_resident(desc.get_or_insert(whole), resident, u32::from(own.MipLevels));
                }
                // A host copy without the title's largest mips: the view's
                // mips are its own, that many lower.
                if let Some(d) = desc.as_mut() {
                    shift_view_levels(crate::resource::dropped(resource), d);
                }
                // SAFETY: a live texture and a slot of our heap.
                unsafe {
                    device.CreateShaderResourceView(
                        texture,
                        desc.as_ref().map(std::ptr::from_ref),
                        at,
                    )
                };
                Some(Some(texture.clone()))
            } else if let (Some((data, len)), Some(desc)) =
                (crate::resource::buffer_bytes(resource), desc)
            {
                buffer_srv(data as u64, len, srv_desc(desc), at)
            } else {
                None
            }
        }
        View::Unordered(resource, _counter, desc) => {
            let resource = *resource as crate::object::This;
            let Some(texture) = crate::resource::host_resource(resource) else {
                return match (crate::resource::buffer_bytes(resource), desc) {
                    (Some((data, len)), Some(desc)) => {
                        buffer_uav(data as u64, len, uav_desc(desc), at)
                    }
                    _ => None,
                };
            };
            // The first such views of depth textures are logged as the
            // title describes them (format, dimension, then mip, slices and
            // plane): they say which planes shaders write, and as what.
            if crate::resource::depth_and_unordered(resource) {
                static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 512 {
                    tracing::info!(
                        ?resource,
                        view = ?desc,
                        "unordered access view of a depth texture"
                    );
                }
            }
            let mut desc = desc.as_ref().map(uav_desc);
            // A depth texture's unordered access is its colour twin's: the
            // host gives a depth texture none.
            let twin = crate::resource::twin_resource(resource)
                .and_then(|twin| Some((twin, twin_uav(twin, desc.as_ref())?)))
                // A view of the depth plane of a texture with a stencil
                // plane: of that plane's own twin.
                .or_else(|| {
                    let twin = crate::resource::twin_plane_resource(resource)?;
                    Some((twin, twin_uav(twin, desc.as_ref())?))
                });
            let texture = match twin {
                Some((twin, view)) => {
                    desc = Some(view);
                    twin
                }
                None => texture,
            };
            if !fits(texture, desc.as_mut().map(|d| &mut d.Format), true) {
                return None;
            }
            // SAFETY: a live texture and a slot of our heap.
            unsafe {
                device.CreateUnorderedAccessView(
                    texture,
                    None::<&ID3D12Resource>,
                    desc.as_ref().map(std::ptr::from_ref),
                    at,
                )
            };
            Some(Some(texture.clone()))
        }
        View::Sampler(words) => {
            // SAFETY: 13 words in the host's layout.
            let desc = unsafe {
                std::ptr::from_ref(words)
                    .cast::<D3D12_SAMPLER_DESC>()
                    .read_unaligned()
            };
            // SAFETY: a slot of our sampler heap.
            unsafe { device.CreateSampler(&desc, at) };
            Some(None)
        }
        View::ConsoleSampler(words) => {
            let desc = crate::sampler::console(words, crate::sampler::aniso_point_mip(device));
            // SAFETY: a slot of our sampler heap.
            unsafe { device.CreateSampler(&desc, at) };
            Some(None)
        }
        View::Placed(unordered, resource, desc) => {
            if let Some(view) = placed_texture(view) {
                return make(&view, at);
            }
            const BUFFER: u32 = 1;
            if resource[0] != BUFFER {
                return None;
            }
            // The resource description's width, after the alignment.
            let len = u64::from(resource[4]) | u64::from(resource[5]) << 32;
            let address = u64::from(desc[10]) | u64::from(desc[11]) << 32;
            let words: &[u32; 10] = desc[..10].try_into().unwrap_or(&[0; 10]);
            // An empty title buffer: nothing to view, and no host copy to
            // look for (#70: such views were in flight at device removals).
            if len == 0 {
                return None;
            }
            if *unordered {
                buffer_uav(address, len, uav_desc(words), at)
            } else {
                buffer_srv(address, len, srv_desc(words), at)
            }
        }
    }
}

/// Writes of a slot with another view of the placed texture it showed,
/// since the start: the slot kept showing the old view.
pub(crate) static KEPT_VIEWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Whether a slot written with another view of the placed texture it
/// shows keeps showing the old one until the new is made;
/// `VICTORY_KEEP_VIEWS=0` makes it null at once, as before.
fn keep_views() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_KEEP_VIEWS").is_none_or(|v| v != "0"))
}

/// Whether both are shader views of one texture the title placed in
/// memory, the same address and the same resource description, that
/// differ in their mips at most. The old view shows a sharper or blurrier
/// picture of the same thing; one of another array slice, plane or format
/// (a video's ring of pictures) would show an older picture instead.
fn same_placed_texture(a: &View, b: &View) -> bool {
    match (a, b) {
        (View::Placed(false, resource, desc), View::Placed(false, other, other_desc)) => {
            resource[0] != 1
                && resource == other
                && desc[10..12] == other_desc[10..12]
                && (0..10).all(|i| desc[i] == other_desc[i] || mip_word(desc[1], i))
        }
        _ => false,
    }
}

/// Whether word `i` of a shader view's description (the host's layout) is
/// one of its mip range or its least detail clamp, for view dimension
/// `dimension`. Word 3 is padding; the union starts at word 4.
fn mip_word(dimension: u32, i: usize) -> bool {
    let clamp = match D3D12_SRV_DIMENSION(dimension as i32) {
        D3D12_SRV_DIMENSION_TEXTURE1D
        | D3D12_SRV_DIMENSION_TEXTURE3D
        | D3D12_SRV_DIMENSION_TEXTURECUBE => 6,
        D3D12_SRV_DIMENSION_TEXTURE2D => 7,
        D3D12_SRV_DIMENSION_TEXTURE1DARRAY | D3D12_SRV_DIMENSION_TEXTURECUBEARRAY => 8,
        D3D12_SRV_DIMENSION_TEXTURE2DARRAY => 9,
        // Multisampled views have no mips.
        _ => return i == 3,
    };
    matches!(i, 3..=5) || i == clamp
}

/// A shader or unordered access view of a texture the title placed in
/// memory itself, as a view of the resource made for that memory (on first
/// use).
pub(crate) fn placed_texture(view: &View) -> Option<View> {
    const BUFFER: u32 = 1;
    const TARGET_DEPTH_UNORDERED: u32 = 0x7;
    let &View::Placed(unordered, mut resource, ref desc) = view else {
        return None;
    };
    if resource[0] == BUFFER {
        return None;
    }
    let address = u64::from(desc[10]) | u64::from(desc[11]) << 32;
    let words = desc[..10].try_into().ok();
    if unordered {
        let texture = crate::resource::uav_texture_at(address, &resource, desc[0])?;
        Some(View::Unordered(texture as usize, 0, words))
    } else {
        // Shader views placed in memory take their texels either from title
        // memory or from `TARGETS_AT` (`resource::alias_of`): clear target
        // flags the title left on the resource description so this view
        // never replaces that target in `TARGETS_AT`.
        resource[12] &= !TARGET_DEPTH_UNORDERED;
        let texture = crate::resource::texture_at(address, &resource)?;
        Some(View::Shader(texture as usize, words))
    }
}

/// A null view of the kind `view` is, for a slot whose view the host
/// cannot make: unordered access (to a buffer or a 2D texture) or a shader
/// view. Writes to it are dropped, reads give zeros.
fn null_for(view: &View, at: D3D12_CPU_DESCRIPTOR_HANDLE) {
    let Some(host) = host() else { return };
    let is_buffer = |r: &usize| crate::resource::buffer_bytes(*r as crate::object::This).is_some();
    // Unordered views: whether a buffer, and the title's view dimension.
    let (buffer, dimension) = match view {
        View::Unordered(r, _, desc) => (
            is_buffer(r) || desc.is_some_and(|d| d[1] == 1),
            desc.map(|d| d[1]),
        ),
        View::Placed(true, resource, desc) => (resource[0] == 1, Some(desc[1])),
        // Shader views of buffers read nothing through a null buffer view:
        // a null 2D view where the shader expects a buffer is a mismatch
        // the hardware does not survive in general (GPU-based validation
        // reports it).
        View::Shader(r, desc) if is_buffer(r) || desc.is_some_and(|d| d[1] == 1) => {
            return null_buffer_view(at);
        }
        View::Placed(false, resource, _) if resource[0] == 1 => return null_buffer_view(at),
        // Textures: a null view of the title's dimension (1D, arrays, 3D,
        // cubes), not always 2D.
        View::Shader(_, Some(desc)) => return null_like(desc, at),
        View::Placed(false, _, desc) => {
            return null_like(desc[..10].try_into().unwrap_or(&[0; 10]), at);
        }
        View::Constant(..) => {
            let desc = D3D12_CONSTANT_BUFFER_VIEW_DESC::default();
            // SAFETY: a slot of our heap; a zero location makes a null view.
            return unsafe { host.device.CreateConstantBufferView(Some(&desc), at) };
        }
        _ => return null_view(at),
    };
    let desc = if buffer {
        D3D12_UNORDERED_ACCESS_VIEW_DESC {
            Format: DXGI_FORMAT_R32_UINT,
            ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
            ..Default::default()
        }
    } else {
        null_uav_desc(dimension)
    };
    // SAFETY: a slot of our heap; a null resource is allowed with a desc.
    unsafe {
        host.device.CreateUnorderedAccessView(
            None::<&ID3D12Resource>,
            None::<&ID3D12Resource>,
            Some(&desc),
            at,
        )
    };
}

/// A null texture UAV of the title's view dimension (1D, 1D or 2D arrays,
/// 3D), 2D if it names none: a 2D null view where the shader writes a 3D
/// texture is a mismatch GPU-based validation reports (the world's wind
/// field, written by one of the title's compute programs, 2026-11-24).
fn null_uav_desc(dimension: Option<u32>) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
    let mut desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
        Format: DXGI_FORMAT_R32_UINT,
        ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
        ..Default::default()
    };
    match dimension.map(|d| D3D12_UAV_DIMENSION(d as i32)) {
        Some(D3D12_UAV_DIMENSION_TEXTURE1D) => {
            desc.ViewDimension = D3D12_UAV_DIMENSION_TEXTURE1D;
        }
        Some(D3D12_UAV_DIMENSION_TEXTURE1DARRAY) => {
            desc.ViewDimension = D3D12_UAV_DIMENSION_TEXTURE1DARRAY;
            desc.Anonymous.Texture1DArray = D3D12_TEX1D_ARRAY_UAV {
                ArraySize: 1,
                ..Default::default()
            };
        }
        Some(D3D12_UAV_DIMENSION_TEXTURE2DARRAY) => {
            desc.ViewDimension = D3D12_UAV_DIMENSION_TEXTURE2DARRAY;
            desc.Anonymous.Texture2DArray = D3D12_TEX2D_ARRAY_UAV {
                ArraySize: 1,
                ..Default::default()
            };
        }
        Some(D3D12_UAV_DIMENSION_TEXTURE3D) => {
            desc.ViewDimension = D3D12_UAV_DIMENSION_TEXTURE3D;
            desc.Anonymous.Texture3D = D3D12_TEX3D_UAV {
                WSize: u32::MAX,
                ..Default::default()
            };
        }
        _ => {}
    }
    desc
}

/// A buffer view that reads zeros (raw, as translated shaders mostly read
/// buffers).
fn null_buffer_view(at: D3D12_CPU_DESCRIPTOR_HANDLE) {
    let Some(host) = host() else { return };
    let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_R32_TYPELESS,
        ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
        Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
        Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
            Buffer: D3D12_BUFFER_SRV {
                NumElements: 1,
                Flags: D3D12_BUFFER_SRV_FLAG_RAW,
                ..Default::default()
            },
        },
    };
    // SAFETY: a slot of our heap; a null resource is allowed with a desc.
    unsafe {
        host.device
            .CreateShaderResourceView(None::<&ID3D12Resource>, Some(&desc), at)
    };
}

/// A texture view that reads zeros with the dimension (and mip and array
/// counts) of the title's description `words`; 2D if it names none.
fn null_like(words: &[u32; 10], at: D3D12_CPU_DESCRIPTOR_HANDLE) {
    const TEXTURES: std::ops::RangeInclusive<u32> = 2..=10;
    if !TEXTURES.contains(&words[1]) {
        return null_view(at);
    }
    let Some(host) = host() else { return };
    // One mip, one slice or cube, plane 0: fields a null view still checks.
    let mut w = [0u32; 10];
    w[1] = words[1];
    match words[1] {
        6 => {}
        7 => w[5] = 1,
        3 | 5 | 10 => (w[5], w[7]) = (1, 1),
        _ => w[5] = 1,
    }
    let mut desc = srv_desc(&w);
    desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.Shader4ComponentMapping = D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
    // SAFETY: a slot of our heap; a null resource is allowed with a desc.
    unsafe {
        host.device
            .CreateShaderResourceView(None::<&ID3D12Resource>, Some(&desc), at)
    };
}

/// A view that reads zeros, for slots whose view the host cannot make.
/// shortcut: slots with no view at all (kind unknown) get this null 2D
/// view; null buffer views there did not stop the world's low-address
/// page faults (2026-11-24, run w10), so they are not the cause.
fn null_view(at: D3D12_CPU_DESCRIPTOR_HANDLE) {
    let Some(host) = host() else { return };
    let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
        Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
        Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
            Texture2D: D3D12_TEX2D_SRV {
                MipLevels: 1,
                ..Default::default()
            },
        },
    };
    // SAFETY: a slot of our heap; a null resource is allowed with a desc.
    unsafe {
        host.device
            .CreateShaderResourceView(None::<&ID3D12Resource>, Some(&desc), at)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directly_indexed_buffer_ranges_follow_the_heap_lifetime() {
        use std::sync::Arc;

        let mut views = Box::new(BufferViews::new());
        views.written(3, true);
        views.written(7, true);
        let (slots, mut pass) = views.begin_pass();
        assert_eq!(slots, [3, 7]);
        let shadow = Arc::new(());
        let weak = Arc::downgrade(&shadow);
        pass.by_slot.insert(7, (shadow, 16, 48));
        drop(pass);
        if rolling_rescan() {
            // A sixteenth of the slots at each pass, with the changed ones.
            for phase in 1..16u32 {
                views.written(3, true);
                let mut slots = views.begin_pass().0;
                slots.sort_unstable();
                assert_eq!(slots, if phase == 7 { vec![3, 7] } else { vec![3] }, "{phase}");
                assert!(weak.upgrade().is_some(), "slot 7 is a buffer view still");
            }
            // Slot 7 is no buffer view any more: its range goes at its share.
            views.noted(7, false);
            for _ in 0..16 {
                drop(views.begin_pass());
            }
            assert!(weak.upgrade().is_none());
        } else {
            for _ in 1..16 {
                views.written(3, true);
                assert_eq!(views.begin_pass().0, [3]);
                assert!(weak.upgrade().is_some());
            }
            let (slots, pass) = views.begin_pass();
            drop(pass);
            assert_eq!(slots, [3, 7]);
            assert!(weak.upgrade().is_none());
        }
        let (_, mut pass) = views.begin_pass();

        let shadow = Arc::new(());
        let weak = Arc::downgrade(&shadow);
        pass.by_slot.insert(7, (shadow, 32, 48));
        drop(pass);
        // The new heap's tracking state occupies exactly the old address.
        *views = BufferViews::new();
        assert!(weak.upgrade().is_none());
        let (_, pass) = views.begin_pass();
        assert!(pass.by_slot.is_empty());
        assert_eq!(pass.passes, 1);
    }

    /// The unsigned twin of a 16-bit float or signed format has its layout,
    /// and only buffer views in such a format are shown as another view.
    #[test]
    fn only_16_bit_float_and_signed_buffer_views_get_an_unsigned_twin() {
        for (format, unsigned) in [(10, 12), (14, 12), (34, 36), (38, 36), (54, 57), (59, 57)] {
            assert_eq!(half_format(format), Some(unsigned));
            assert_eq!(
                crate::resource::element(format),
                crate::resource::element(unsigned),
                "format {format}"
            );
        }
        // The unsigned formats themselves, normalised 16-bit formats, and
        // wider and narrower ones.
        for other in [0, 12, 36, 57, 11, 13, 35, 37, 56, 58, 42, 17, 30, 61] {
            assert_eq!(half_format(other), None, "format {other}");
        }
        const BUFFER: u32 = 1;
        let words = |format: u32, dimension: u32| [format, dimension, 5768, 0, 0, 0, 64, 0, 0, 0];
        // R16G16B16A16_FLOAT: a buffer of halves a program moves as they are.
        assert_eq!(
            half_view(&View::Unordered(7, 0, Some(words(10, BUFFER)))),
            Some(View::Unordered(7, 0, Some(words(12, BUFFER))))
        );
        assert_eq!(
            half_view(&View::Shader(7, Some(words(54, BUFFER)))),
            Some(View::Shader(7, Some(words(57, BUFFER))))
        );
        let mut placed = [0u32; 16];
        placed[..10].copy_from_slice(&words(38, BUFFER));
        placed[10] = 0x1234_0000;
        let mut twin = placed;
        twin[0] = 36;
        let memory = [BUFFER, 0, 0, 0, 512, 0, 1, 0x1_0001, 0, 1, 0, 1, 0, 0];
        assert_eq!(
            half_view(&View::Placed(true, memory, placed)),
            Some(View::Placed(true, memory, twin))
        );
        // The title's kind (R16G16B16A16_UINT and R16_UINT buffers), a
        // texture view (dimension 4: 2D), a view without a description, a
        // constant buffer: the title's own.
        assert_eq!(
            half_view(&View::Unordered(7, 0, Some(words(12, BUFFER)))),
            None
        );
        assert_eq!(half_view(&View::Shader(7, Some(words(57, BUFFER)))), None);
        assert_eq!(half_view(&View::Unordered(7, 0, Some(words(10, 4)))), None);
        assert_eq!(half_view(&View::Shader(7, None)), None);
        assert_eq!(half_view(&View::Constant(0x1000, 256)), None);
    }

    /// The buffer views through which the integers a shader moves halves
    /// as are not what the console gives are named; the title's are not.
    #[test]
    fn views_whose_values_the_console_would_convert_are_named() {
        const BUFFER: u32 = 1;
        let words = |format: u32, dimension: u32| [format, dimension, 5768, 0, 0, 0, 64, 0, 0, 0];
        for exact in [10, 12, 14, 34, 36, 38, 54, 57, 59, 25, 30, 50, 62] {
            let view = View::Unordered(7, 0, Some(words(exact, BUFFER)));
            assert_eq!(half_inexact(&view), None, "format {exact}");
        }
        // R32_FLOAT, R32_UINT, R16G16B16A16_UNORM, R8G8B8A8_UNORM, R8_SINT,
        // and a structured or raw view (no format).
        for inexact in [41, 42, 11, 28, 64, 0] {
            let view = View::Shader(7, Some(words(inexact, BUFFER)));
            assert_eq!(half_inexact(&view), Some(inexact));
        }
        let mut placed = [0u32; 16];
        placed[..10].copy_from_slice(&words(41, BUFFER));
        let memory = [BUFFER, 0, 0, 0, 512, 0, 1, 0x1_0001, 0, 1, 0, 1, 0, 0];
        assert_eq!(half_inexact(&View::Placed(true, memory, placed)), Some(41));
        // Not a buffer view.
        assert_eq!(half_inexact(&View::Shader(7, Some(words(41, 4)))), None);
        assert_eq!(half_inexact(&View::Constant(0x1000, 256)), None);
    }

    /// The two ways a translated `buffer_load_format_d16_xyzw` and
    /// `buffer_store_format_d16_xyzw` have moved a typed buffer's 16-bit
    /// channels, on the host device, over a buffer that holds every 16-bit
    /// pattern. As unsigned integers through an unsigned view (the title's
    /// own R16G16B16A16_UINT, or the twin `half_view` gives a float one)
    /// every pattern comes back as it was. By way of floats, as before
    /// (load as floats, each to its half's bits; each half widened and
    /// stored as a float): through the title's integer view that is
    /// outside what D3D12 defines, and what the device gives is printed,
    /// not checked (1 pattern of 65,536 on the cards tried: the network
    /// that moves the title's faces computed zeros).
    #[test]
    fn halves_cross_an_unsigned_buffer_view_unchanged() {
        const MOVES: &str = r#"
#define RS "DescriptorTable(UAV(u0, numDescriptors=2))"
#ifdef BY_FLOATS
RWBuffer<float4> from : register(u0);
RWBuffer<float4> to : register(u1);
#else
RWBuffer<uint4> from : register(u0);
RWBuffer<uint4> to : register(u1);
#endif
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void BY_FLOATS_OR_NOT(uint3 id : SV_DispatchThreadID) {
#ifdef BY_FLOATS
    uint4 h = f32tof16(from[id.x]);
#else
    uint4 h = from[id.x] & 0xffff;
#endif
    uint low = h.x | h.y << 16;
    uint high = h.z | h.w << 16;
#ifdef BY_FLOATS
    to[id.x] = float4(f16tof32(low), f16tof32(low >> 16), f16tof32(high), f16tof32(high >> 16));
#else
    to[id.x] = uint4(low & 0xffff, low >> 16, high & 0xffff, high >> 16);
#endif
}
"#;
        const ELEMENTS: usize = 1 << 14;
        const BYTES: u64 = ELEMENTS as u64 * 8;
        const TITLES: u32 = 12; // R16G16B16A16_UINT
        let Some(host) = host() else {
            return; // no host GPU
        };
        let heap: ID3D12DescriptorHeap = {
            let desc = D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: 2,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            };
            // SAFETY: plain heap creation.
            unsafe { host.device.CreateDescriptorHeap(&desc) }.expect("a view heap")
        };
        // How many of the 65,536 patterns come back as they were, moved by
        // the program `by_floats` or not, through views in `format`.
        let moved = |by_floats: bool, format: u32| -> usize {
            let source = if by_floats {
                MOVES
                    .replace("BY_FLOATS_OR_NOT", "main")
                    .replace("#ifdef BY_FLOATS", "#if 1")
            } else {
                MOVES
                    .replace("BY_FLOATS_OR_NOT", "main")
                    .replace("#ifdef BY_FLOATS", "#if 0")
            };
            let code = crate::host::compile(
                &source,
                windows::core::s!("main"),
                windows::core::s!("cs_5_1"),
            )
            .expect("the test's shader");
            let (from, filled) = host.shared_buffer(BYTES).expect("a buffer");
            let (to, result) = host.shared_buffer(BYTES).expect("a buffer");
            // SAFETY: the shader's own root signature and plain creation
            // calls; both mappings hold `BYTES` bytes; the views are of
            // live buffers, in slots of the heap made above; an open list.
            unsafe {
                let root: ID3D12RootSignature = host
                    .device
                    .CreateRootSignature(0, &code)
                    .expect("a root signature");
                let pso: ID3D12PipelineState = host
                    .device
                    .CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
                        pRootSignature: std::mem::transmute_copy(&root),
                        CS: D3D12_SHADER_BYTECODE {
                            pShaderBytecode: code.as_ptr().cast(),
                            BytecodeLength: code.len(),
                        },
                        ..Default::default()
                    })
                    .expect("a pipeline");
                let filled = std::slice::from_raw_parts_mut(filled.cast::<u16>(), ELEMENTS * 4);
                for (pattern, half) in filled.iter_mut().enumerate() {
                    *half = pattern as u16;
                }
                std::ptr::write_bytes(result, 0xee, BYTES as usize);
                let view = D3D12_UNORDERED_ACCESS_VIEW_DESC {
                    Format: DXGI_FORMAT(format as i32),
                    ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
                    Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                        Buffer: D3D12_BUFFER_UAV {
                            NumElements: ELEMENTS as u32,
                            ..Default::default()
                        },
                    },
                };
                let start = heap.GetCPUDescriptorHandleForHeapStart();
                let step = host
                    .device
                    .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
                    as usize;
                host.device.CreateUnorderedAccessView(
                    &from,
                    None::<&ID3D12Resource>,
                    Some(&view),
                    start,
                );
                let second = D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: start.ptr + step,
                };
                host.device.CreateUnorderedAccessView(
                    &to,
                    None::<&ID3D12Resource>,
                    Some(&view),
                    second,
                );
                let list = host.new_list().expect("a host list");
                list.list.SetDescriptorHeaps(&[Some(heap.clone())]);
                list.list.SetComputeRootSignature(&root);
                list.list.SetPipelineState(&pso);
                list.list
                    .SetComputeRootDescriptorTable(0, heap.GetGPUDescriptorHandleForHeapStart());
                list.list.Dispatch(ELEMENTS as u32 / 64, 1, 1);
                list.close();
                host.execute(&[&list]);
                assert!(
                    host.device.GetDeviceRemovedReason().is_ok(),
                    "the device, after views in format {format}"
                );
                let result = std::slice::from_raw_parts(result.cast::<u16>(), ELEMENTS * 4);
                (0..=u16::MAX)
                    .filter(|&p| result[usize::from(p)] == p)
                    .count()
            }
        };
        // A float view of halves is shown as the title's kind of view.
        assert_eq!(half_format(10), Some(TITLES));
        let unsigned = moved(false, TITLES);
        let by_floats = moved(true, TITLES);
        eprintln!(
            "halves through an R16G16B16A16_UINT buffer view: {unsigned} of 65536 patterns came back unchanged moved as unsigned integers, {by_floats} moved by way of floats"
        );
        assert_eq!(
            unsigned, 65_536,
            "patterns unchanged, moved as unsigned integers"
        );
    }

    /// A null texture UAV keeps the title's dimension (3D for the world's
    /// wind field), and is 2D when the title names none.
    #[test]
    fn null_uavs_keep_the_titles_dimension() {
        let d = null_uav_desc(Some(8));
        assert_eq!(d.ViewDimension, D3D12_UAV_DIMENSION_TEXTURE3D);
        // SAFETY: the member the dimension names.
        assert_eq!(unsafe { d.Anonymous.Texture3D.WSize }, u32::MAX);
        let d = null_uav_desc(Some(5));
        assert_eq!(d.ViewDimension, D3D12_UAV_DIMENSION_TEXTURE2DARRAY);
        // SAFETY: the member the dimension names.
        assert_eq!(unsafe { d.Anonymous.Texture2DArray.ArraySize }, 1);
        let dimension = |d| null_uav_desc(d).ViewDimension;
        assert_eq!(dimension(Some(2)), D3D12_UAV_DIMENSION_TEXTURE1D);
        assert_eq!(dimension(Some(4)), D3D12_UAV_DIMENSION_TEXTURE2D);
        assert_eq!(dimension(None), D3D12_UAV_DIMENSION_TEXTURE2D);
        // Multisampled UAVs need a cap; they stay 2D.
        assert_eq!(dimension(Some(6)), D3D12_UAV_DIMENSION_TEXTURE2D);
    }

    /// What the texture code asks of the spans (levels on demand): where
    /// the next texture starts inside a span, the lowest level a view of a
    /// texture shows, and a texture's slots.
    #[test]
    fn spans_say_the_next_start_and_the_lowest_level_viewed() {
        let mut spans = Spans::default();
        // A 512x512 texture of 0x56000 bytes viewed from levels 3 and 1,
        // and the next one 0x6000 after it (run 188).
        spans.note(1, 0x1_0000, 0x1_0000 + 0x5_6000, &[0; 14], true);
        spans.shows(1, 3);
        spans.note(2, 0x1_0000, 0x1_0000 + 0x5_6000, &[0; 14], true);
        spans.shows(2, 1);
        spans.note(3, 0x1_6000, 0x1_6000 + 0x5_6000, &[0; 14], true);
        assert_eq!(spans.next_start(0x1_0000, 0x1_0000 + 0x5_6000), Some(0x1_6000));
        assert_eq!(spans.next_start(0x1_6000, 0x1_6000 + 0x5_6000), None);
        assert_eq!(spans.first_viewed(0x1_0000), Some(1));
        assert_eq!(spans.first_viewed(0x1_6000), None);
        assert_eq!(spans.slots_at(0x1_0000), vec![1, 2]);
        // The view from level 1 goes: the texture is viewed from level 3.
        spans.forget(2);
        assert_eq!(spans.first_viewed(0x1_0000), Some(3));
    }

    /// Views of one placed texture are told from views of others by the
    /// address and the resource description, not by what they show.
    #[test]
    fn views_of_one_placed_texture_are_told_by_address_and_description() {
        let texture = [3, 0, 0, 0, 512, 0, 512, 0x8_0001, 98, 1, 0, 0x105, 0, 0];
        let view = |resource: [u32; 14], first_mip: u32, address: u32| {
            let mut desc = [0u32; 16];
            (desc[0], desc[1], desc[4], desc[5], desc[10], desc[11]) = (98, 4, first_mip, 8 - first_mip, address, 0xb);
            View::Placed(false, resource, desc)
        };
        let low = view(texture, 3, 0x1000_0000);
        // more mips of the same texture
        assert!(same_placed_texture(&low, &view(texture, 0, 0x1000_0000)));
        // another address, another description, a buffer, an unordered view
        assert!(!same_placed_texture(&low, &view(texture, 3, 0x2000_0000)));
        let mut wider = texture;
        wider[4] = 1024;
        assert!(!same_placed_texture(&low, &view(wider, 3, 0x1000_0000)));
        let mut buffer = texture;
        buffer[0] = 1;
        assert!(!same_placed_texture(&view(buffer, 3, 0x1000_0000), &view(buffer, 3, 0x1000_0000)));
        let View::Placed(_, resource, desc) = low.clone() else { unreachable!() };
        assert!(!same_placed_texture(&low, &View::Placed(true, resource, desc)));
        assert!(!same_placed_texture(&low, &View::Shader(0, None)));
        // Of the same texture, but not only its mips: another plane or
        // format; a clamp of the least detail is a mip range still.
        let with = |word: usize, value: u32| {
            let mut other = desc;
            other[word] = value;
            View::Placed(false, resource, other)
        };
        assert!(!same_placed_texture(&low, &with(6, 1)), "plane");
        assert!(!same_placed_texture(&low, &with(0, 99)), "format");
        assert!(
            same_placed_texture(&low, &with(7, 2.0f32.to_bits())),
            "clamp"
        );
        // Array slices of one texture (a ring of video pictures).
        let slice = |first: u32| {
            let mut array = desc;
            (array[1], array[6], array[7]) = (5, first, 1);
            View::Placed(false, resource, array)
        };
        assert!(same_placed_texture(&slice(2), &slice(2)));
        assert!(!same_placed_texture(&slice(2), &slice(3)));
    }

    /// A constant buffer view larger than the host takes is cut to what it
    /// takes: the device, which such a view removes, stays.
    #[test]
    fn an_oversized_constant_buffer_view_leaves_the_device_alive() {
        let Some(host) = host() else {
            return; // no host GPU
        };
        let Some(heap) = HostHeap::new(false, 4) else {
            return;
        };
        // SAFETY: a plain device query.
        let alive = || unsafe { host.device.GetDeviceRemovedReason() }.is_ok();
        // Made first: an allocation that fails on a removed device must not
        // end the test early.
        let (buffer, _) = host.upload_buffer(1 << 20).expect("an upload buffer");
        let memory = vec![0u8; 2 << 20];
        let at = (memory.as_ptr() as u64).next_multiple_of(64 << 10);
        for size in [969_472u32, 896_256] {
            assert!(make(&View::Constant(at, size), heap.cpu(0)).is_some());
            assert!(alive(), "a view of {size} bytes");
        }
        // SAFETY: a live upload buffer.
        heap.constant_at(1, unsafe { buffer.GetGPUVirtualAddress() }, 969_472);
        assert!(alive(), "a view of the list's own memory");
        assert_eq!(constant_bytes(480), 512);
        assert_eq!(constant_bytes(64 << 10), 64 << 10);
        assert_eq!(constant_bytes(u32::MAX), 64 << 10);
    }

    /// A null 3D UAV made on the real device leaves it alive.
    #[test]
    fn null_3d_uav_is_accepted_by_the_device() {
        let Some(host) = host() else {
            return; // no host GPU
        };
        let Some(heap) = HostHeap::new(false, 4) else {
            return;
        };
        let view = View::Unordered(0, 0, Some([28, 8, 0, 0, 0, 0, 0, 0, 0, 0]));
        null_for(&view, heap.cpu(0));
        // SAFETY: a plain device query.
        assert!(unsafe { host.device.GetDeviceRemovedReason() }.is_ok());
    }

    /// A typed buffer view: `count` elements of `format` from `first`.
    fn typed_view(format: DXGI_FORMAT, first: u64, count: u32) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
        D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: format,
            ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Buffer: D3D12_BUFFER_SRV {
                    FirstElement: first,
                    NumElements: count,
                    StructureByteStride: 0,
                    Flags: D3D12_BUFFER_SRV_FLAG_NONE,
                },
            },
        }
    }

    /// Which views need a copy of their own (#62): typed and structured
    /// ones not on an element of their copy, not raw windows, not those on
    /// an element, not views of no known element size.
    #[test]
    fn views_off_their_copys_elements_are_told_apart() {
        let rgb = typed_view(DXGI_FORMAT_R32G32B32_FLOAT, 0, 22);
        assert!(off_element(4, &rgb), "12-byte elements 4 bytes in");
        assert!(off_element(16, &rgb));
        assert!(!off_element(24, &rgb), "on an element");
        assert!(!off_element(0, &rgb));
        assert!(!off_element(6, &typed_view(DXGI_FORMAT_R16_FLOAT, 0, 8)));
        assert!(off_element(6, &typed_view(DXGI_FORMAT_R32_FLOAT, 0, 8)));
        assert!(!off_element(4, &typed_view(DXGI_FORMAT_UNKNOWN, 0, 8)), "no element size");
        let mut structured = typed_view(DXGI_FORMAT_UNKNOWN, 0, 10);
        structured.Anonymous.Buffer.StructureByteStride = 20;
        assert!(!off_element(32, &structured), "a raw window");
        assert!(off_element(8, &structured), "no raw window, off its elements");
        assert!(off_element(40, &structured), "on an element, yet not on 16 bytes: raw on its own copy");
        assert!(!off_element(80, &structured), "a raw window again");
        structured.Anonymous.Buffer.StructureByteStride = 10;
        structured.Anonymous.Buffer.NumElements = 3;
        assert!(!off_element(20, &structured), "30 bytes are not whole dwords: it stays structured");
        // The title's own raw views: the host's begin on 16 bytes.
        let mut raw = typed_view(DXGI_FORMAT_R32_TYPELESS, 2, 32);
        raw.Anonymous.Buffer.Flags = D3D12_BUFFER_SRV_FLAG_RAW;
        assert!(off_element(0, &raw), "8 bytes into its copy");
        assert!(!off_element(8, &raw), "8 + 8: on 16 bytes");
        assert!(off_element(4, &raw));
        raw.Anonymous.Buffer.FirstElement = 0;
        assert!(!off_element(32, &raw));
        raw.Anonymous.Buffer.NumElements = 0;
        assert!(!off_element(8, &raw), "of no elements: left null as before");
    }

    /// A typed view that begins off an element of its memory's copy is made
    /// on a copy of its own range, at element 0, and the slot's copy is the
    /// one `buffer_copy` names, so the heap pass brings it up to date (#62:
    /// such views were left null and read zeros). A view on an element
    /// still uses the buffer's copy.
    #[test]
    fn a_typed_view_off_its_copys_elements_gets_its_own_copy() {
        if host().is_none() {
            return; // no host GPU
        }
        let Some(heap) = HostHeap::new(false, 4) else {
            return;
        };
        const RESOURCE: usize = 0x62_0062;
        let title = vec![0u8; 8192];
        let start = title.as_ptr() as u64;
        crate::shadow::register(RESOURCE, start, title.len() as u64);
        // The title's views of memory 4 bytes into the buffer: 22 of 12
        // bytes (264, as in #62) from element 1, then the same view's
        // memory on an element of the buffer's copy.
        let placed = |base: u64, desc: &D3D12_SHADER_RESOURCE_VIEW_DESC| {
            // SAFETY: 40 bytes in the host's layout, as `srv_desc` reads them.
            let words: [u32; 10] = unsafe { std::ptr::from_ref(desc).cast::<[u32; 10]>().read_unaligned() };
            let mut resource = [0u32; 14];
            resource[0] = 1; // a buffer
            resource[4] = 1024; // its width
            let mut view = [0u32; 16];
            view[..10].copy_from_slice(&words);
            view[10] = base as u32;
            view[11] = (base >> 32) as u32;
            View::Placed(false, resource, view)
        };
        let desc = typed_view(DXGI_FORMAT_R32G32B32_FLOAT, 1, 22);
        let base = start + 4;
        let kept = buffer_srv(base, 1024, desc, heap.cpu(0)).expect("a view, not left null");
        let (copy, address, len) = buffer_copy(&placed(base, &desc)).unwrap();
        assert_eq!((address, len), (base + 12, 22 * 12));
        assert_eq!(copy.span(), (base + 12, 22 * 12), "a copy of the view's range alone");
        assert_eq!(kept.as_ref(), Some(&copy.buffer), "the slot shows the copy the pass updates");

        let kept = buffer_srv(start + 24, 1024, desc, heap.cpu(1)).expect("a view");
        let (copy, ..) = buffer_copy(&placed(start + 24, &desc)).unwrap();
        assert_eq!(copy.span(), (start, title.len() as u64), "the buffer's own copy");
        assert_eq!(kept.as_ref(), Some(&copy.buffer));
        crate::shadow::unregister(RESOURCE, start);
    }

    /// A record that names the buffer view its slot already shows leaves the
    /// slot as it is; one whose slot was made null, or another view, is
    /// made. The pass over buffer views does not mark the slots it makes
    /// for the next pass.
    #[test]
    fn a_record_of_the_view_a_slot_shows_leaves_the_slot_as_it_is() {
        use std::sync::atomic::Ordering::Relaxed;
        let Some(heap) = HostHeap::new(false, 4) else {
            return;
        };
        const RESOURCE: usize = 0x62_0063;
        let title = vec![0u8; 8192];
        let start = title.as_ptr() as u64;
        crate::shadow::register(RESOURCE, start, title.len() as u64);
        let placed = |count: u32| {
            let desc = typed_view(DXGI_FORMAT_R32_UINT, 0, count);
            // SAFETY: 40 bytes in the host's layout, as `srv_desc` reads them.
            let words: [u32; 10] = unsafe { std::ptr::from_ref(&desc).cast::<[u32; 10]>().read_unaligned() };
            let mut resource = [0u32; 14];
            resource[0] = 1; // a buffer
            resource[4] = 8192; // its width
            let mut view = [0u32; 16];
            view[..10].copy_from_slice(&words);
            view[10] = start as u32;
            view[11] = (start >> 32) as u32;
            View::Placed(false, resource, view)
        };
        let record = |view: View| {
            let [entry, generation] = super::super::words_for(view);
            ([super::super::VIEW_TAG, entry, generation], u64::from(generation) << 32 | u64::from(entry))
        };
        let slot = 2;
        let view = placed(64);
        let sig = same_view_signature(&view).expect("a buffer view");
        let (words, key) = record(view.clone());
        let handle = words.as_ptr() as u64;
        let _in_pass = HeapPass::enter();
        assert_eq!(heap.mirror(slot, handle, true), Some(view.clone()));
        assert!(heap.kept_at(slot).is_some());
        assert!(heap.shows_same(slot, sig, &view));
        assert!(!heap.buffers.changed.lock().unwrap().contains(&slot), "not marked for the next pass");
        assert!(heap.buffers.slots.lock().unwrap().contains(&slot));

        // The same view again under another name: left as it was.
        let before = SAME_VIEWS[0].load(Relaxed);
        heap.made.lock().unwrap()[slot as usize] = 0;
        heap.made_fast_set(slot, 0);
        assert_eq!(heap.mirror(slot, handle, true), Some(view.clone()));
        assert!(SAME_VIEWS[0].load(Relaxed) > before);
        assert_eq!(heap.made.lock().unwrap()[slot as usize], key);
        assert!(heap.shows_same(slot, sig, &view));

        // Made null: not the same any more, and made again.
        heap.keep(slot, None);
        assert!(!heap.shows_same(slot, sig, &view));
        heap.made.lock().unwrap()[slot as usize] = 0;
        heap.made_fast_set(slot, 0);
        assert_eq!(heap.mirror(slot, handle, true), Some(view.clone()));
        assert!(heap.shows_same(slot, sig, &view));

        // Another view of the same memory: another fingerprint.
        let other = placed(32);
        assert_ne!(same_view_signature(&other), Some(sig));
        let (words, _) = record(other.clone());
        assert_eq!(heap.mirror(slot, words.as_ptr() as u64, true), Some(other.clone()));
        assert!(!heap.shows_same(slot, sig, &view));
        drop(_in_pass);

        // A record copied in as bytes: the buffer view is made at once.
        let (words, key) = record(view.clone());
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let copied = 3;
        assert!(heap.mirror_copied(copied, words.as_ptr() as u64, &bytes));
        assert_eq!(heap.made.lock().unwrap()[copied as usize], key);
        assert!(heap.kept_at(copied).is_some(), "made, not left null");
        if sync_ranges() {
            // Its range is given to the pass, which does not look at it.
            assert!(!heap.buffers.changed.lock().unwrap().contains(&copied));
            let (_, ranges) = heap.buffers.begin_pass();
            assert!(ranges.by_slot.contains_key(&copied), "the pass has its range");
        } else {
            assert!(heap.buffers.changed.lock().unwrap().contains(&copied), "the pass looks at it");
        }
        crate::shadow::unregister(RESOURCE, start);
    }

    /// Dynamic slots stay taken until given back, and runs do not wrap.
    #[test]
    fn a_structured_view_made_raw_passes_the_host_check() {
        // 100 elements of 20 bytes from element 8, in a copy that starts 32
        // bytes before the view's memory: bytes 192..2192.
        let (first, count) = raw_window(32, 8, 100, 20).unwrap();
        assert_eq!((first, count), (48, 500));
        assert!(buffer_view_ok(DXGI_FORMAT_R32_TYPELESS, first, count, 0, true, 4096));
        // the format a raw view must name; the structured one's is refused
        assert!(!buffer_view_ok(DXGI_FORMAT_UNKNOWN, first, count, 0, true, 4096));
        // past the buffer, not on 16 bytes, no elements
        assert!(!buffer_view_ok(DXGI_FORMAT_R32_TYPELESS, first, count, 0, true, 2000));
        assert!(raw_window(4, 0, 100, 20).is_none());
        assert!(!buffer_view_ok(DXGI_FORMAT_R32_TYPELESS, 0, 0, 0, true, 4096));
        // a structured view the host takes as it is
        assert!(buffer_view_ok(DXGI_FORMAT_UNKNOWN, 1, 10, 12, false, 4096));
    }

    /// The queue of textures picked by index serves the slots looked at
    /// before first, then the newest write and the oldest in turns; a
    /// slot whose memory is not there is looked at again after
    /// 1, 2, 4 frames up to the limit; and what the pass reports about a
    /// slot written again meanwhile is dropped.
    #[test]
    fn the_texture_queue_serves_the_newest_writes_first() {
        let mut q = TextureQueue::default();
        // Slots 10..18 written in frames 1..9, slot 5 last.
        for i in 0..8u32 {
            q.written(10 + i, u64::from(i) + 1, 0, false);
        }
        q.written(5, 9, 0, false);
        let order: Vec<u32> = q.due(9, 100).into_iter().map(|d| d.0).collect();
        // newest, oldest, newest, oldest, and so on
        assert_eq!(order, [5, 10, 17, 11, 16, 12, 15, 13, 14]);
        assert_eq!(q.due(9, 2).len(), 2);
        assert_eq!(q.census(9), [9, 9, 0, 0, 8, 9, 0, 0, 0]);
        // Slot 17's memory is not mapped: again next frame, then in 2, 4, 4.
        let write = |q: &TextureQueue, index: u32| q.due(u64::MAX, 100).iter().find(|d| d.0 == index).unwrap().1;
        let w17 = write(&q, 17);
        let mut frame = 9;
        for wait in [1, 2, 4, 4] {
            q.later(17, w17, frame, 4);
            assert!(!q.due(frame + wait - 1, 100).iter().any(|d| d.0 == 17));
            frame += wait;
            // Due again, and ahead of every slot never looked at.
            assert_eq!(q.due(frame, 100)[0].0, 17);
        }
        assert_eq!(q.census(9)[2], 1);
        q.done(17, w17);
        assert!(!q.due(u64::MAX, 100).iter().any(|d| d.0 == 17));
        // Uploaded with part of its memory there: kept, with what was there.
        let w16 = write(&q, 16);
        q.partly(16, w16, 12, 4096);
        assert_eq!(q.mapped(16), 4096);
        assert_eq!(q.due(12, 100)[0].0, 16);
        // The title writes slot 16 again while the pass works on it: what
        // the pass then says about the old write changes nothing.
        q.written(16, 13, 0, false);
        q.done(16, w16);
        q.later(16, w16, 13, 4);
        assert_eq!(q.mapped(16), 0);
        assert_eq!(q.due(13, 100)[0].0, 16);
        // A draw that makes the host slot (`seen`) keeps what the queue knows,
        let w15 = write(&q, 15);
        q.partly(15, w15, 20, 8192);
        q.written(15, 14, 0, true);
        assert_eq!(q.mapped(15), 8192);
        // and queues a slot the queue did not hold.
        q.written(99, 14, 0, true);
        assert!(q.due(14, 100).iter().any(|d| d.0 == 99));
        // A write due later (a table's slot) is not handed out before.
        q.written(98, 14, 16, false);
        assert!(!q.due(15, 100).iter().any(|d| d.0 == 98));
        assert!(q.due(16, 100).iter().any(|d| d.0 == 98));
    }

    /// [`TextureQueue::due`] as it was before it selected what it sorts:
    /// every class sorted whole, the list cut at `max` at the end.
    fn due_full_sort(q: &TextureQueue, frame: u64, max: usize) -> Vec<(u32, u64)> {
        let mut again = Vec::new();
        let mut fresh = Vec::new();
        let mut stale = Vec::new();
        for (&index, w) in q.slots.iter().filter(|(_, w)| w.due <= frame && w.relook.is_none()) {
            if w.stale {
                stale.push((w.due, index, w.write));
                continue;
            }
            let class = if w.looks > 0 || w.mapped > 0 { &mut again } else { &mut fresh };
            class.push((w.written, index, w.write));
        }
        again.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.2));
        fresh.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.2));
        stale.sort_unstable_by_key(|&(found, _, write)| (found, write));
        let mut order: Vec<(u32, u64)> = again.iter().map(|&(_, index, write)| (index, write)).collect();
        let (mut new, mut old) = (0, fresh.len());
        while new < old && order.len() < max {
            order.push((fresh[new].1, fresh[new].2));
            new += 1;
            if new < old && order.len() < max {
                old -= 1;
                order.push((fresh[old].1, fresh[old].2));
            }
        }
        order.extend(stale.iter().map(|&(_, index, write)| (index, write)));
        order.truncate(max);
        order
    }

    /// A queue of `slots` slots written in turn, their slot numbers out of
    /// the order of their writes as in a streamed heap; every
    /// `repeat_every`th looked at before, every fifth of the others found
    /// changed after an upload (in frames 0 to 2), every thirteenth waiting
    /// for a look and every eleventh due from frame 1.
    fn due_fixture(slots: u32, repeat_every: u32) -> TextureQueue {
        let mut q = TextureQueue::default();
        for i in 0..slots {
            // An odd multiplier gives unique slots in the 64K table.
            let index = i.wrapping_mul(32749) & 0xffff;
            q.written(index, u64::from(i), u64::from(i % 11 == 0), false);
            let w = q.slots.get_mut(&index).unwrap();
            if i.is_multiple_of(repeat_every) {
                w.looks = 1;
            } else if i % 5 == 0 {
                w.stale = true;
                w.due = u64::from(i % 3);
            } else if i % 13 == 0 {
                w.relook = Some(Relook { at: (0, 4096), print: 1, looks: 0, uploaded: 0 });
            }
        }
        q
    }

    /// The selection hands out the same slots, in the same order, as the
    /// full sorts: retries of every mix, slots due later, changed ones,
    /// limits from none to more than the queue holds.
    #[test]
    fn bounded_texture_selection_matches_full_sort() {
        for repeat_every in [1, 3, 17] {
            let q = due_fixture(73, repeat_every);
            for frame in [0, 1, 2, 3] {
                for max in 0..=80 {
                    assert_eq!(q.due(frame, max), due_full_sort(&q, frame, max), "every {repeat_every}th a retry, frame {frame}, at most {max}");
                }
            }
        }
    }

    /// The two over a full streaming queue: 40,000 slots, 4,096 taken
    /// (`cargo test ... -- --ignored --nocapture`).
    #[test]
    #[ignore = "benchmark of a full streaming queue"]
    fn bench_bounded_texture_selection() {
        use std::hint::black_box;
        use std::time::Instant;
        for repeat_every in [2, 17] {
            let q = due_fixture(40_000, repeat_every);
            assert_eq!(q.due(3, 4096), due_full_sort(&q, 3, 4096));
            let mut samples = [Vec::new(), Vec::new()];
            for _ in 0..15 {
                for (i, elapsed) in samples.iter_mut().enumerate() {
                    let began = Instant::now();
                    let order = if i == 0 { due_full_sort(black_box(&q), 3, 4096) } else { black_box(&q).due(3, 4096) };
                    black_box(order);
                    elapsed.push(began.elapsed());
                }
            }
            samples.iter_mut().for_each(|v| v.sort_unstable());
            eprintln!(
                "40,000 slots, every {repeat_every}th a retry, 4,096 taken: full sort {:?}, selection {:?}",
                samples[0][7], samples[1][7],
            );
        }
    }

    /// Slot `index` uploaded whole in `frame` from its own memory
    /// (`0x1000 * index`, 4 KiB) with fingerprint `print`, as the pass does.
    /// Returns the write number the pass was handed.
    fn uploaded(q: &mut TextureQueue, index: u32, frame: u64, print: u64) -> u64 {
        q.written(index, frame, 0, false);
        let write = q.writes;
        assert!(!q.uploaded_whole(index, write, frame, (0x1000 * u64::from(index), 4096), print));
        write
    }

    /// Slots uploaded whole that only wait for looks at their memory do not
    /// count as waiting for an upload (the sweep's gate).
    #[test]
    fn slots_waiting_for_looks_are_not_uploads_waiting() {
        let mut q = TextureQueue::default();
        for index in 0..5 {
            uploaded(&mut q, index, 10, 7);
        }
        q.written(9, 10, 11, false);
        assert_eq!(q.slots.len(), 6);
        assert_eq!(q.uploads_waiting(), 1);
    }

    /// The looks at a texture the title filled after its upload come at
    /// 4, 12, 40, 120 and 400 frames, further apart for each change.
    #[test]
    fn looks_at_uploaded_textures_follow_the_schedule() {
        let waits = |changes| (0..6).map(|looks| relook_wait(looks, changes)).collect::<Vec<_>>();
        assert_eq!(waits(0), [Some(4), Some(12), Some(40), Some(120), Some(400), None]);
        assert_eq!(waits(1), [Some(8), Some(24), Some(80), Some(240), Some(400), None]);
        assert_eq!(waits(3), [Some(32), Some(96), Some(320), Some(400), Some(400), None]);
        // However many changes there were, no wait is longer than 400
        // frames.
        for changes in [7, 8, 100, u8::MAX] {
            assert_eq!(waits(changes), [Some(400), Some(400), Some(400), Some(400), Some(400), None]);
        }
        // A texture that holds still: its looks, then it leaves the queue.
        let mut q = TextureQueue::default();
        let write = uploaded(&mut q, 7, 100, 1);
        let mut looked = Vec::new();
        for frame in 100..=800 {
            assert!(q.due(frame, 8).is_empty(), "a slot uploaded whole is not the pass's");
            for (index, write, _) in q.relooks_due(frame, 8) {
                looked.push(frame);
                q.relooked(index, write, frame, false);
            }
        }
        assert_eq!(looked, [104, 116, 156, 276, 676]);
        assert!(q.slots.is_empty());
        q.relooked(7, write, 800, true);
        assert!(q.slots.is_empty());
    }

    /// A slot never uploaded goes ahead of every re-upload a look asked for,
    /// whatever the size of the pass, and slots uploaded whole that wait for
    /// a look are neither due for the pass nor counted as waiting for their
    /// memory.
    #[test]
    fn slots_never_uploaded_go_ahead_of_every_look_and_re_upload() {
        let mut q = TextureQueue::default();
        // 300 slots uploaded in one burst at frame 10.
        for i in 0..300u32 {
            uploaded(&mut q, 1000 + i, 10, 7);
        }
        assert!(q.due(u64::MAX, 4096).is_empty());
        assert_eq!(q.census(13), [300, 0, 0, 0, 0, 0, 300, 0, 0]);
        assert_eq!(q.census(14), [300, 0, 0, 0, 0, 0, 300, 0, 300]);
        // 50 slots written at frame 14, never uploaded.
        for i in 0..50u32 {
            q.written(i, 14, 0, false);
        }
        // Looks find two thirds of the burst changed.
        let looks = q.relooks_due(14, 4096);
        assert_eq!(looks.len(), 300);
        for (index, write, look) in looks {
            assert_eq!(look.uploaded, 10);
            q.relooked(index, write, 14, index % 3 != 0);
        }
        let order = q.due(14, 4096);
        assert_eq!(order.len(), 250);
        assert!(order[..50].iter().all(|d| d.0 < 50), "never uploaded first");
        assert!(order[50..].iter().all(|d| d.0 >= 1000));
        for max in 0..=260 {
            assert_eq!(q.due(14, max), order[..max.min(250)]);
        }
        // never uploaded, waiting for a look, changed and waiting, looks due
        assert_eq!(q.census(14)[5..], [50, 100, 200, 0]);
        assert_eq!(q.census(14)[..4], [350, 250, 0, 0]);
        // The ones that were unchanged are looked at again in 12 frames.
        assert_eq!(q.relooks_due(25, 4096).len(), 0);
        assert_eq!(q.relooks_due(26, 4096).len(), 100);
    }

    /// The black face: a texture whose memory the title filled after the
    /// upload is uploaded again in the frame of the first look that finds it
    /// so, as soon as no slot never uploaded is ahead of it.
    #[test]
    fn a_texture_filled_after_its_upload_is_uploaded_again_in_a_few_frames() {
        let mut q = TextureQueue::default();
        let write = uploaded(&mut q, 7, 10, 1);
        assert!(q.relooks_due(13, 8).is_empty());
        let looks = q.relooks_due(14, 8);
        assert_eq!(looks.iter().map(|l| (l.0, l.2.looks)).collect::<Vec<_>>(), [(7, 0)]);
        q.relooked(7, write, 14, true);
        assert_eq!(q.due(14, 8), [(7, write)]);
        assert_eq!(q.census(14)[5..], [0, 0, 1, 0]);
        // Uploaded again: the next look comes after twice the first wait.
        assert!(q.uploaded_whole(7, write, 14, (0x7000, 4096), 2));
        assert!(q.due(u64::MAX, 8).is_empty());
        assert!(q.relooks_due(21, 8).is_empty());
        assert_eq!(q.relooks_due(22, 8)[0].2.print, 2);
    }

    /// A texture the title keeps writing is not uploaded again at every
    /// look: the waits double, up to 400 frames (every 4 before: 500
    /// uploads in these 2,000 frames), and the slot is still looked at.
    #[test]
    fn a_texture_that_always_changes_does_not_loop_at_full_rate() {
        let mut q = TextureQueue::default();
        uploaded(&mut q, 3, 0, 1);
        let mut uploads = vec![0u64];
        for frame in 1..=2000u64 {
            for (index, write, _) in q.relooks_due(frame, 8) {
                q.relooked(index, write, frame, true);
            }
            for (index, write) in q.due(frame, 8) {
                assert!(q.uploaded_whole(index, write, frame, (0x3000, 4096), frame + 1));
                uploads.push(frame);
            }
        }
        assert_eq!(uploads, [0, 4, 12, 28, 60, 124, 252, 508, 908, 1308, 1708]);
        assert!(q.slots[&3].relook.is_some());
    }

    /// A slot found unmapped or only partly mapped forgets what the last
    /// look compared with (the memory it held is not the memory it holds),
    /// and a write or a remap of its memory starts it over.
    #[test]
    fn a_slot_waiting_for_its_memory_forgets_the_look_it_had() {
        let mut q = TextureQueue::default();
        let w1 = uploaded(&mut q, 1, 0, 9);
        let w2 = uploaded(&mut q, 2, 0, 9);
        uploaded(&mut q, 3, 0, 9);
        let w4 = uploaded(&mut q, 4, 0, 9);
        // A look finds slot 4 changed, then its memory is gone.
        q.relooked(4, w4, 4, true);
        assert_eq!(q.census(4)[7], 1);
        q.later(4, w4, 4, 16);
        q.later(1, w1, 4, 16);
        q.partly(2, w2, 5, 4096);
        // Slot 3 is written again by the title.
        q.written(3, 4, 4, false);
        assert!(q.relooks_due(u64::MAX, 8).is_empty());
        let order: Vec<u32> = q.due(100, 8).into_iter().map(|d| d.0).collect();
        // looked at before (waiting for memory or partly) first, then the new write
        assert_eq!(order.len(), 4);
        assert_eq!(order[3], 3);
        assert_eq!(q.census(100)[2..4], [2, 1]);
        assert_eq!(q.census(100)[5..], [3, 0, 0, 0]);
        // Memory remapped under an uploaded slot starts it over.
        let w5 = uploaded(&mut q, 5, 10, 9);
        q.changed(5, 12, 13);
        assert!(q.slots[&5].relook.is_none() && q.slots[&5].changes == 0);
        assert_eq!(q.due(13, 8).iter().filter(|d| d.0 == 5).count(), 1);
        // What the pass reports about the old write is dropped.
        assert!(!q.uploaded_whole(5, w5, 13, (0x5000, 4096), 1));
        assert!(q.slots[&5].relook.is_none());
    }

    /// A frame's looks are bounded in number and in time, the ones that
    /// waited longest first, and the fingerprint runs without the queue's
    /// lock.
    #[test]
    fn looks_per_frame_are_bounded() {
        let queue = Mutex::new(TextureQueue::default());
        {
            let mut q = queue.lock().unwrap();
            for i in 0..100u32 {
                uploaded(&mut q, i, 0, 5);
            }
        }
        let seen = Mutex::new(Vec::new());
        let print = |at: u64, len: u64| {
            assert_eq!(len, 4096);
            // The lock is free while a fingerprint is worked out.
            assert!(queue.try_lock().is_ok());
            seen.lock().unwrap().push(at / 0x1000);
            5
        };
        let forever = std::time::Duration::from_secs(60);
        let (looks, found) = relook(&queue, 4, 8, forever, print, |_, _| panic!("unchanged"));
        assert_eq!((looks, found), (8, 0));
        assert_eq!(*seen.lock().unwrap(), (0..8).collect::<Vec<_>>());
        // The next frame's looks go on with the ones not looked at yet.
        let (looks, _) = relook(&queue, 5, 8, forever, print, |_, _| panic!("unchanged"));
        assert_eq!(looks, 8);
        assert_eq!(seen.lock().unwrap()[8..], (8..16).collect::<Vec<_>>());
        // A change is told, once, and the slot waits for its upload.
        let mut told = Vec::new();
        let (looks, found) = relook(&queue, 5, 1, forever, |_, _| 6, |slot, look| told.push((slot, look.print)));
        assert_eq!((looks, found), (1, 1));
        assert_eq!(told, [(16, 5)]);
        assert_eq!(queue.lock().unwrap().due(5, 8).iter().map(|d| d.0).collect::<Vec<_>>(), [16]);
        // The time a frame may spend ends the looks early.
        let slow = |_: u64, _: u64| {
            std::thread::sleep(std::time::Duration::from_millis(4));
            5
        };
        let (looks, _) = relook(&queue, 5, 64, std::time::Duration::from_millis(10), slow, |_, _| {});
        assert!((1..64).contains(&looks), "{looks}");
        // Looks not made stay due: all slots are looked at in the end.
        let mut frame = 5;
        while queue.lock().unwrap().census(frame)[8] > 0 {
            frame += 1;
            assert!(frame < 5000);
            relook(&queue, frame, 8, forever, |_, _| 5, |_, _| {});
        }
    }

    /// Slots held back for lack of room on the card are not handed out
    /// until their frame, keep their place, are not counted as waiting
    /// for their memory, and all come back at once when the card has room;
    /// a slot written again meanwhile is the title's new write.
    #[test]
    fn slots_held_for_lack_of_room_come_back_when_there_is_room() {
        let mut q = TextureQueue::default();
        for i in 0..4u32 {
            q.written(20 + i, 1, 0, false);
        }
        let due = q.due(1, 100);
        assert_eq!(due.len(), 4);
        // The card is full: slots 20 and 21 are held back until frame 101.
        for &(index, write) in due.iter().filter(|d| d.0 < 22) {
            q.hold(index, write, 101);
        }
        let now: Vec<u32> = q.due(2, 100).into_iter().map(|d| d.0).collect();
        assert!(!now.contains(&20) && !now.contains(&21));
        assert!(now.contains(&22) && now.contains(&23));
        assert_eq!(q.census(2)[2], 0, "held slots do not wait for their memory");
        // Due again at their frame, in the order they had.
        assert_eq!(q.due(101, 100), due);
        // Slot 21 is written again while held: due as written.
        q.written(21, 3, 0, false);
        assert!(q.due(3, 100).iter().any(|d| d.0 == 21));
        // Room again at frame 5: slot 20 is due at once, and only it was
        // still held as it was.
        assert_eq!(q.room(5), 1);
        assert!(q.due(5, 100).iter().any(|d| d.0 == 20));
        assert_eq!(q.room(6), 0);
    }

    /// Textures of which only the smallest levels are held, packed into
    /// one another's address range (the three of run 188: 0x2c000 bytes
    /// each at full size, their starts 0x2000 and 0x4000 apart): writing
    /// any of them, in any order and again, leaves the others.
    #[test]
    fn packed_neighbours_stay() {
        let mut spans = Spans::default();
        assert_eq!(spans.rule, Rule::Start);
        let at = 0xb_86bf_8000u64;
        let (a, b, c) = (at, at + 0x2000, at + 0x6000);
        assert!(spans.note(105_998, a, a + 0x2c000, &shape(a, a + 0x2c000), true).is_empty());
        assert!(spans.note(106_004, b, b + 0x2c000, &shape(b, b + 0x2c000), true).is_empty());
        assert!(spans.note(108_521, c, c + 0x2c000, &shape(c, c + 0x2c000), true).is_empty());
        // The first written again (as a title does when levels arrive),
        // from another slot too: its span reaches over the other two.
        assert!(spans.note(7, a, a + 0x2c000, &shape(a, a + 0x2c000), true).is_empty());
        assert!(spans.note(106_004, b, b + 0x2c000, &shape(b, b + 0x2c000), true).is_empty());
        let mut all = spans.slots_in(0, u64::MAX);
        all.sort_unstable();
        assert_eq!(all, [7, 105_998, 106_004, 108_521]);
        // A draw makes a slot of the second texture: it is one of them.
        assert!(spans.note(9, b, b + 0x2c000, &shape(b, b + 0x2c000), false).is_empty());
        assert_eq!(spans.by_slot.get(&9), Some(&(b, b + 0x2c000)));
    }

    /// A texture written where another starts replaces it: at the same
    /// address with another length, or inside the block of its smallest
    /// levels. The packed neighbours of the one replaced stay.
    #[test]
    fn a_texture_written_at_the_start_of_another_replaces_it() {
        let mut spans = Spans::default();
        let at = 0x7_0000_0000u64;
        assert!(spans.note(1, at, at + 0x2c000, &shape(at, at + 0x2c000), true).is_empty());
        assert!(spans.note(2, at + 0x2000, at + 0x2e000, &shape(at + 0x2000, at + 0x2e000), true).is_empty());
        // Another length from the same start: slot 1 is stale, slot 2 not.
        assert_eq!(spans.note(3, at, at + 0xb000, &shape(at, at + 0xb000), true), [1]);
        // A start inside the first 4 KiB of slot 3's texture: stale too.
        assert_eq!(spans.note(4, at + 0x800, at + 0x1800, &shape(at + 0x800, at + 0x1800), true), [3]);
        // And the other way: a texture that starts just before one.
        assert_eq!(spans.note(5, at + 0x1800, at + 0x4800, &shape(at + 0x1800, at + 0x4800), true), [2]);
        let mut left = spans.slots_in(0, u64::MAX);
        left.sort_unstable();
        assert_eq!(left, [4, 5]);
        // Exactly 4 KiB apart: both there.
        assert!(spans.note(6, at + 0x2800, at + 0x3800, &shape(at + 0x2800, at + 0x3800), true).is_empty());
        // A draw's view that clashes is left out, and nothing is stale.
        assert!(spans.note(8, at + 0x900, at + 0x2900, &shape(at + 0x900, at + 0x2900), false).is_empty());
        assert_eq!(spans.by_slot.get(&8), None);
        assert_eq!(spans.by_slot.len(), 3);
    }

    /// A slot written with another texture leaves its old span; the same
    /// view written again changes nothing.
    #[test]
    fn a_slot_rewritten_moves_to_its_new_span() {
        let mut spans = Spans::default();
        assert!(spans.note(1, 0x10000, 0x3c000, &shape(0x10000, 0x3c000), true).is_empty());
        assert!(spans.note(2, 0x12000, 0x3e000, &shape(0x12000, 0x3e000), true).is_empty());
        assert!(spans.note(1, 0x10000, 0x3c000, &shape(0x10000, 0x3c000), true).is_empty());
        assert_eq!(spans.slots_in(0x10000, 0x10001), [1]);
        // Slot 1 now views a texture far away.
        assert!(spans.note(1, 0x90000, 0x91000, &shape(0x90000, 0x91000), true).is_empty());
        assert_eq!(spans.slots_in(0x10000, 0x10001), [] as [u32; 0]);
        assert_eq!(spans.slots_in(0x12000, 0x12001), [2]);
        assert_eq!(spans.slots_in(0x90000, 0x90001), [1]);
        spans.forget(1);
        spans.forget(2);
        assert!(spans.spans.is_empty() && spans.by_slot.is_empty());
    }

    /// Memory the runtime maps, unmaps or writes names the textures that
    /// are there: among packed neighbours the one whose held levels the
    /// range lies in (it ends where the next one starts), and the ones
    /// that start inside the range.
    #[test]
    fn memory_changed_under_packed_neighbours_names_the_ones_there() {
        let mut spans = Spans::default();
        let at = 0x7_0000_0000u64;
        assert!(spans.note(1, at, at + 0x2c000, &shape(at, at + 0x2c000), true).is_empty());
        assert!(spans.note(2, at + 0x2000, at + 0x2e000, &shape(at + 0x2000, at + 0x2e000), true).is_empty());
        assert!(spans.note(3, at + 0x6000, at + 0x32000, &shape(at + 0x6000, at + 0x32000), true).is_empty());
        // Inside the first one's held levels.
        assert_eq!(spans.slots_in(at + 0x1000, at + 0x2000), [1]);
        // Inside the second's: not the first, which ends where it starts.
        assert_eq!(spans.slots_in(at + 0x3000, at + 0x4000), [2]);
        // Past the last start: the last one.
        assert_eq!(spans.slots_in(at + 0x20000, at + 0x21000), [3]);
        // A range over all of them.
        assert_eq!(spans.slots_in(at, at + 0x8000), [1, 2, 3]);
        // Past the end of all: none.
        assert_eq!(spans.slots_in(at + 0x32000, at + 0x33000), [] as [u32; 0]);
    }

    /// A write names the textures that hold its bytes: the one that
    /// starts where the write starts, not the neighbour before it, whose
    /// held levels end there.
    #[test]
    fn a_write_at_the_start_of_a_packed_texture_names_it_alone() {
        let mut spans = Spans::default();
        let at = 0x7_0000_0000u64;
        assert!(spans.note(1, at, at + 0x2c000, &[0; 14], true).is_empty());
        assert!(spans.note(2, at + 0x2000, at + 0x2e000, &[0; 14], true).is_empty());
        assert!(spans.note(3, at + 0x6000, at + 0x32000, &[0; 14], true).is_empty());
        // A request that fills the second texture's held levels.
        assert_eq!(spans.slots_in(at + 0x2000, at + 0x6000), [1, 2]);
        assert_eq!(spans.slots_holding(at + 0x2000, at + 0x6000), [2]);
        // From inside the first one's held levels on: both.
        assert_eq!(spans.slots_holding(at + 0x1000, at + 0x3000), [1, 2]);
        // A whole 64 KiB page: every texture that starts in it.
        assert_eq!(spans.slots_holding(at, at + 0x10000), [1, 2, 3]);
        // Past the last start, the last one; past all, or no bytes, none.
        assert_eq!(spans.slots_holding(at + 0x20000, at + 0x21000), [3]);
        assert_eq!(spans.slots_holding(at + 0x32000, at + 0x33000), [] as [u32; 0]);
        assert_eq!(spans.slots_holding(at + 0x2000, at + 0x2000), [] as [u32; 0]);
    }

    /// A write behind the guards' back under a placed texture: a slot
    /// waiting for its first upload is left alone, one uploaded before is
    /// counted, and made due two frames on when that is asked for. (The
    /// ranges are handed over here, not through the list all heaps share.)
    #[test]
    fn a_page_write_makes_an_uploaded_texture_slot_due_when_asked() {
        let Some(heap) = HostHeap::new(false, 8) else {
            return; // no host GPU
        };
        let slots = |at: u64| -> Vec<u32> { heap.texture_slots_due(at, 8).0.into_iter().map(|d| d.0).collect() };
        // 64x64 BC7, one mip, at an address no test and no title uses.
        let at = 0x7fed_0000_0000u64;
        let mut desc = [0u32; 16];
        (desc[10], desc[11]) = (at as u32, (at >> 32) as u32);
        let view = View::Placed(false, [3, 0, 0, 0, 64, 0, 64, 0x1_0001, 98, 1, 0, 0x105, 0, 0], desc);
        assert!(heap.note_buffer(1, Some(&view), false).is_empty());
        let written = [(at, at + 0x1000)];
        // Waiting for its first upload: held, not uploaded, nothing done.
        assert_eq!(heap.slots_written_under(&written, 50, true), [1, 0, 0]);
        let (queued, write) = heap.texture_slots_due(u64::MAX, 8).0[0];
        assert_eq!(queued, 1);
        heap.texture_slot_done(1, write, None);
        assert_eq!(slots(u64::MAX), []);
        // Uploaded and out of the queue. Memory elsewhere: nothing.
        assert_eq!(heap.slots_written_under(&[(at + 0x10_0000, at + 0x10_1000)], 60, true), [0, 0, 0]);
        // Under it, counted only: it stays out of the queue.
        assert_eq!(heap.slots_written_under(&written, 60, false), [1, 1, 0]);
        assert_eq!(slots(u64::MAX), []);
        // Under it, asked for: due two frames on, not before.
        assert_eq!(heap.slots_written_under(&written, 60, true), [1, 1, 1]);
        assert_eq!(slots(61), []);
        assert_eq!(slots(62), [1]);
    }

    /// `VICTORY_STALE_TEXTURES=0`: nothing is ever stale, and a view that
    /// overlaps others is left out of the spans, as before.
    #[test]
    fn never_stale_leaves_overlapping_views_out() {
        let mut spans = Spans { rule: Rule::Never, ..Default::default() };
        assert!(spans.note(1, 0x1000, 0x3000, &shape(0x1000, 0x3000), true).is_empty());
        assert!(spans.note(2, 0x2000, 0x4000, &shape(0x2000, 0x4000), true).is_empty());
        assert!(spans.note(3, 0x1000, 0x2000, &shape(0x1000, 0x2000), true).is_empty());
        assert_eq!(spans.slots_in(0, u64::MAX), [1]);
    }

    /// The rule until run 188 (`VICTORY_STALE_TEXTURES=overlap`): a view
    /// written over memory that older views describe otherwise makes
    /// those stale; views of the same span are one texture's.
    #[test]
    fn a_newer_view_of_other_memory_makes_the_older_stale() {
        let mut spans = Spans { rule: Rule::Overlap, ..Default::default() };
        // Three textures side by side, the first in two slots.
        assert!(spans.note(1, 0x1000, 0x3000, &shape(0x1000, 0x3000), true).is_empty());
        assert!(spans.note(2, 0x1000, 0x3000, &shape(0x1000, 0x3000), true).is_empty());
        assert!(spans.note(3, 0x3000, 0x4000, &shape(0x3000, 0x4000), true).is_empty());
        assert!(spans.note(4, 0x8000, 0x9000, &shape(0x8000, 0x9000), true).is_empty());
        // The same slot written with the same span again: nothing changes.
        assert!(spans.note(2, 0x1000, 0x3000, &shape(0x1000, 0x3000), true).is_empty());
        assert_eq!(spans.slots_in(0x2000, 0x3800), [1, 2, 3]);
        assert_eq!(spans.slots_in(0x4000, 0x8000), [] as [u32; 0]);
        // A draw makes a slot whose view overlaps them: left out, nothing stale.
        assert!(spans.note(9, 0x2000, 0x3800, &shape(0x2000, 0x3800), false).is_empty());
        assert_eq!(spans.slots_in(0x2000, 0x3800), [1, 2, 3]);
        // The title writes a texture over the end of the first and all of
        // the second: their three slots are stale.
        let mut stale = spans.note(5, 0x2000, 0x4000, &shape(0x2000, 0x4000), true);
        stale.sort_unstable();
        assert_eq!(stale, [1, 2, 3]);
        assert_eq!(spans.slots_in(0, u64::MAX), [5, 4]);
        // Another start in the same memory: stale in turn. Another length
        // from the same start too.
        assert_eq!(spans.note(6, 0x2800, 0x3000, &shape(0x2800, 0x3000), true), [5]);
        assert_eq!(spans.note(7, 0x2800, 0x2c00, &shape(0x2800, 0x2c00), true), [6]);
        // A slot that moves to another texture leaves its span behind.
        assert!(spans.note(7, 0x8000, 0x9000, &shape(0x8000, 0x9000), true).is_empty());
        assert_eq!(spans.slots_in(0x2800, 0x2c00), [] as [u32; 0]);
        assert_eq!(spans.slots_in(0x8000, 0x8001), [4, 7]);
        spans.forget(4);
        spans.forget(7);
        assert!(spans.spans.is_empty() && spans.by_slot.is_empty());
    }

    /// Views of one texture that show more and fewer of its mips are all
    /// its own, and the next texture may begin where the mips that are
    /// there end.
    #[test]
    fn a_texture_of_which_the_smaller_mips_are_there_ends_behind_them() {
        // As the title has them: 512x512 BC7 of which 64x64 is
        // there, and a 1024x512 BC1 texture 8 KiB on.
        let texture = [3, 0, 0, 0, 512, 0, 512, 0x8_0001, 99, 1, 0, 0x105, 0, 0];
        let next = [3, 0, 0, 0, 1024, 0, 512, 0x8_0001, 71, 1, 0, 0x105, 0, 0];
        let mut spans = Spans::default();
        let at = 0xb_5c04_0000u64;
        let end = |spans: &mut Spans, desc: &[u32; 14], at: u64, first| at + spans.bytes(desc, first).unwrap();
        let a = end(&mut spans, &texture, at, 3);
        assert_eq!(a, at + 0x2000);
        assert!(spans.note(1, at, a, &texture, true).is_empty());
        let b = end(&mut spans, &next, a, 3);
        assert!(b <= a + 0x4000);
        assert!(spans.note(2, a, b, &next, true).is_empty());
        assert_eq!(spans.slots_in(at, b), [1, 2]);
        // The smaller mips alone in another slot: the same texture's.
        let small = end(&mut spans, &texture, at, 5);
        assert!(small < a);
        assert!(spans.note(3, at, small, &texture, true).is_empty());
        assert_eq!(spans.slots_in(at, at + 1), [3, 1]);
        // More of it streamed in, reaching over the next texture, which
        // starts 8 KiB on, outside the first 4 KiB a texture owns: it
        // stays, by the default rule.
        let more = end(&mut spans, &texture, at, 1);
        assert!(more > b);
        assert!(spans.note(1, at, more, &texture, true).is_empty());
        assert_eq!(spans.slots_in(a, a + 1), [1, 2]);
        // By the rule of overlap (`VICTORY_STALE_TEXTURES=overlap`) that
        // one is stale.
        let mut old = Spans { rule: Rule::Overlap, ..Default::default() };
        assert!(old.note(1, at, a, &texture, true).is_empty());
        assert!(old.note(2, a, b, &next, true).is_empty());
        assert_eq!(old.note(1, at, more, &texture, true), [2]);
    }

    /// The filler words of a description (the padding of its structure,
    /// where the title leaves what its stack held) make no other texture:
    /// a view of the texture with other filler in them, showing fewer mips,
    /// is no reason to take its slots for stale; a texture of another size
    /// is.
    #[test]
    fn filler_words_do_not_tell_textures_apart() {
        let texture = [3, 0, 0, 0, 512, 0, 512, 0x8_0001, 99, 1, 0, 0x105, 0, 0];
        let mut filler = texture;
        (filler[1], filler[13]) = (0x7ffe_1234, 0xdead_beef);
        let mut other = texture;
        other[4] = 256;
        let at = 0x4_0000u64;
        let mut spans = Spans::default();
        assert!(spans.note(1, at, at + 0x4000, &texture, true).is_empty());
        assert!(spans.against(at, at + 0x2000, &filler).is_empty());
        assert_eq!(spans.against(at, at + 0x2000, &other), [(at, at + 0x4000)]);
        assert!(spans.note(2, at, at + 0x2000, &filler, true).is_empty());
        assert_eq!(spans.slots_in(at, at + 1), [2, 1]);
        assert_eq!(spans.note(3, at, at + 0x2000, &other, true), [1]);
    }

    /// A resource description of its own for every span.
    fn shape(start: u64, end: u64) -> [u32; 14] {
        let mut desc = [0; 14];
        (desc[0], desc[4], desc[6]) = (3, start as u32, end as u32);
        desc
    }

    /// Whose the memory of a view is: the slot's own span, no other's, or
    /// one that says otherwise.
    #[test]
    fn spans_say_whose_memory_a_view_is() {
        let mut spans = Spans::default();
        assert!(spans.note(1, 0x1000, 0x3000, &shape(0x1000, 0x3000), true).is_empty());
        assert_eq!(spans.claim(1, 0x1000, 0x3000, &shape(0x1000, 0x3000)), (Claim::Own, None));
        // Another slot's view of the same texture, or of memory nothing describes.
        assert_eq!(spans.claim(2, 0x1000, 0x3000, &shape(0x1000, 0x3000)), (Claim::Free, None));
        assert_eq!(spans.claim(2, 0x3000, 0x4000, &shape(0x3000, 0x4000)), (Claim::Free, None));
        // The same start with another length; another start.
        assert_eq!(spans.claim(2, 0x1000, 0x2000, &shape(0x1000, 0x2000)), (Claim::SameStart, Some((0x1000, 0x3000))));
        assert_eq!(spans.claim(2, 0x800, 0x2000, &shape(0x800, 0x2000)), (Claim::Other, Some((0x1000, 0x3000))));
    }

    /// A view that fell out of the queue with its memory there and nothing
    /// to show for it goes back; one a newer view speaks against does not.
    #[test]
    fn a_texture_sweep_finds_views_nothing_brings_up_to_date() {
        let Some(host) = HostHeap::new(false, 8) else {
            return; // no host GPU
        };
        // Below 2^40, as a title's memory is.
        // SAFETY: new memory, left to the process.
        let at = unsafe {
            use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};
            VirtualAlloc(0x7d_4000_0000usize as *const _, 1 << 20, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as u64
        };
        if at == 0 {
            return; // taken
        }
        let mut slots = vec![[0u32; 8]; 8];
        let base = slots.as_mut_ptr() as u64;
        let handle = |index: u32| Some(base + u64::from(index) * 32);
        let resource = [3, 0, 0, 0, 64, 0, 64, 0x1_0001, 98, 1, 0, 0x105, 0, 0];
        let view = |width: u32, address: u64| {
            let mut desc = [0u32; 16];
            (desc[10], desc[11]) = (address as u32, (address >> 32) as u32);
            let mut resource = resource;
            resource[4] = width;
            View::Placed(false, resource, desc)
        };
        super::super::record(handle(1).unwrap(), view(64, at));
        super::super::record(handle(2).unwrap(), view(64, at + 0x8_0000));
        // A newer, wider texture in the second one's memory.
        host.note_buffer(5, Some(&view(128, at + 0x8_0000)), false);
        let queued = || -> Vec<u32> { host.texture_slots_due(u64::MAX, 8).0.into_iter().map(|d| d.0).collect() };
        for slot in queued() {
            host.textures.lock().unwrap().remove(slot);
        }
        let mut counts = [0; 10];
        host.sweep_part(0..8, 0, handle, 50, false, &mut counts);
        assert_eq!(counts, [2, 0, 0, 0, 0, 0, 1, 1, 0, 0]);
        assert_eq!(queued(), []);
        host.sweep_part(0..8, 0, handle, 50, true, &mut counts);
        assert_eq!(queued(), [1]);
    }

    /// With [`sweep_resume`]: a part whose first 128 slots fall out of the
    /// queue at every visit still has its later slots put back (after
    /// DexNom/VIctory b8ff22e).
    #[test]
    fn a_texture_sweep_goes_on_after_the_last_slot_it_put_back() {
        let Some(host) = HostHeap::new(false, 136) else {
            return; // no host GPU
        };
        // Below 2^40, as a title's memory is.
        // SAFETY: new memory, left to the process.
        let at = unsafe {
            use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};
            VirtualAlloc(0x7d_5000_0000usize as *const _, 1 << 20, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as u64
        };
        if at == 0 {
            return; // taken
        }
        let mut slots = vec![[0u32; 8]; 136];
        let base = slots.as_mut_ptr() as u64;
        let handle = |index: u32| Some(base + u64::from(index) * 32);
        let view = |address: u64| {
            let resource = [3, 0, 0, 0, 64, 0, 64, 0x1_0001, 98, 1, 0, 0x105, 0, 0];
            let mut desc = [0u32; 16];
            (desc[10], desc[11]) = (address as u32, (address >> 32) as u32);
            View::Placed(false, resource, desc)
        };
        // All views of one texture: none speaks against another.
        for slot in 0..136 {
            super::super::record(handle(slot).unwrap(), view(at));
        }
        let all_due = || host.texture_slots_due(u64::MAX, 136).0;
        let clear = |due: &[(u32, u64)]| {
            for &(slot, _) in due {
                host.textures.lock().unwrap().remove(slot);
            }
        };
        clear(&all_due());
        SWITCHED_ON.set(true);
        let mut counts = [0; 10];
        let next = host.sweep_part(0..136, 0, handle, 51, true, &mut counts);
        let first = all_due();
        assert_eq!(first.len(), 128);
        assert!(!first.iter().any(|d| d.0 >= 128));
        clear(&first);
        // From the start again, the same 128 would be put back.
        host.sweep_part(0..136, next, handle, 52, true, &mut counts);
        let second = all_due();
        SWITCHED_ON.set(false);
        assert_eq!(second.len(), 128);
        assert!((128..136).all(|slot| second.iter().any(|d| d.0 == slot)));
    }

    /// With [`displaced_recovery`]: views a newer view made stale go back
    /// into the queue once no view claims their memory, only while they
    /// hold the record they showed (after DexNom/VIctory b8ff22e).
    #[test]
    fn an_expired_alias_gives_back_only_unchanged_mapped_views() {
        let Some(heap) = HostHeap::new(false, 8) else { return };
        static PIXELS: [u8; 1 << 20] = [0; 1 << 20];
        let address = PIXELS.as_ptr() as u64;
        let resource = [3, 0, 0, 0, 256, 0, 256, 0x9_0001, 28, 1, 0, 0x105, 0, 0];
        let visible = crate::resource::placed_bytes_from(&resource, 2).unwrap();
        assert!(visible < crate::resource::placed_bytes(&resource).unwrap());
        let record = |address: u64, width: u32, first: u32| {
            let mut shape = resource;
            shape[4] = width;
            let mut view = [0; 16];
            view[..6].copy_from_slice(&[28, 4, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, 0, first, 1]);
            (view[10], view[11]) = (address as u32, (address >> 32) as u32);
            let [i, generation] = super::super::words_for(View::Placed(false, shape, view));
            [VIEW_TAG, i, generation]
        };
        let words = [const { std::cell::Cell::new([0u32; 3]) }; 8];
        let handle = |i: u32| words[i as usize].as_ptr() as u64;
        let recover = |frame| heap.recover_displaced(|i| Some(handle(i)), frame);
        SWITCHED_ON.set(true);
        let old = record(address, 256, 2);
        for i in 1..=3 {
            words[i].set(old);
            heap.mirror(i as u32, handle(i as u32), true).unwrap();
        }
        assert!((1..=3).all(|i| heap.kept_at(i).is_some()));
        let blocker = record(address, 512, 0);
        for i in 4..=5 {
            words[i].set(blocker);
            heap.mirror(i as u32, handle(i as u32), false).unwrap();
        }
        assert!((1..=3).all(|i| heap.kept_at(i).is_none()));
        assert_eq!(recover(40), 0);
        // A title can copy a replacement straight into a displaced slot.
        words[3].set(record(address + visible, 128, 0));
        words[4].set([0; 3]);
        assert!(heap.mirror(4, handle(4), false).is_none());
        assert_eq!(recover(41), 0); // slot 5 still claims the memory
        // A second texture begins where the visible levels end.
        words[6].set(words[3].get());
        heap.mirror(6, handle(6), false).unwrap();
        words[5].set([0; 3]);
        assert!(heap.mirror(5, handle(5), false).is_none());
        assert_eq!(recover(42), 2);
        let queued = |i| heap.textures.lock().unwrap().slots.contains_key(&i);
        assert!(queued(1) && queued(2) && !queued(3));
        // A newer alias between the put-back and the upload takes them out
        // again; once it is gone they come back.
        let stale_write = heap.textures.lock().unwrap().slots[&1].write;
        words[4].set(blocker);
        heap.mirror(4, handle(4), false).unwrap();
        assert!(!queued(1) && !queued(2));
        assert_eq!(recover(43), 0);
        assert!(heap.mirror_queued(1, handle(1), stale_write).is_none());
        assert!(heap.kept_at(1).is_none());
        // A sweep that saw the record before may put it back while the
        // newer view still claims the memory: not made.
        heap.queue_texture(1, true, false, 43);
        let premature = heap.textures.lock().unwrap().slots[&1].write;
        assert!(heap.mirror_queued(1, handle(1), premature).is_none());
        heap.texture_slot_done(1, premature, None);
        words[4].set([0; 3]);
        assert!(heap.mirror(4, handle(4), false).is_none());
        assert_eq!(recover(44), 2);
        for i in 1..=2 {
            let write = heap.textures.lock().unwrap().slots[&i].write;
            assert!(heap.mirror_queued(i, handle(i), write).is_some());
        }
        assert!(!heap.texture_slot_gone(1, stale_write));
        assert_eq!(recover(45), 0);
        SWITCHED_ON.set(false);
        assert!(heap.placed.lock().unwrap().displaced.is_empty());
        assert!((1..=2).all(|i| heap.kept_at(i).is_some()));
        assert!(device_removed().is_none());
    }

    /// With [`stale_after_flights`]: slots a newer view makes stale while
    /// submissions are in the air go on showing their view until those
    /// have finished, and are made null then; one the title writes again
    /// meanwhile is its own again and stays.
    #[test]
    fn a_stale_slot_waits_for_the_submissions_in_the_air() {
        let Some(heap) = HostHeap::new(false, 8) else { return };
        static PIXELS: [u8; 1 << 20] = [0; 1 << 20];
        let address = PIXELS.as_ptr() as u64;
        let resource = [3, 0, 0, 0, 256, 0, 256, 0x9_0001, 28, 1, 0, 0x105, 0, 0];
        let record = |address: u64, width: u32, first: u32| {
            let mut shape = resource;
            shape[4] = width;
            let mut view = [0; 16];
            view[..6].copy_from_slice(&[28, 4, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, 0, first, 1]);
            (view[10], view[11]) = (address as u32, (address >> 32) as u32);
            let [i, generation] = super::super::words_for(View::Placed(false, shape, view));
            [VIEW_TAG, i, generation]
        };
        let words = [const { std::cell::Cell::new([0u32; 3]) }; 8];
        let handle = |i: u32| words[i as usize].as_ptr() as u64;
        let kept = |i: u32| heap.kept_at(i).is_some();
        // Submissions 9 and 10 are in the air.
        STALE_TEST.set(Some((10, 8)));
        let old = record(address, 256, 2);
        for i in 1..=2 {
            words[i].set(old);
            heap.mirror(i as u32, handle(i as u32), true).unwrap();
        }
        assert!(kept(1) && kept(2));
        // A newer view of other memory there: 1 and 2 are stale, and stay.
        words[4].set(record(address, 512, 0));
        heap.mirror(4, handle(4), false).unwrap();
        assert!(kept(1) && kept(2));
        // Submission 9 has finished, 10 not yet.
        STALE_TEST.set(Some((11, 9)));
        heap.stale_later();
        assert!(kept(1) && kept(2));
        // The title writes slot 2 again (a draw reads it): its own.
        heap.mirror(2, handle(2), true).unwrap();
        STALE_TEST.set(Some((11, 10)));
        heap.stale_later();
        assert!(!kept(1));
        assert!(kept(2));
        // Nothing in the air: made null at once, as without the switch.
        words[5].set(record(address, 1024, 0));
        STALE_TEST.set(Some((11, 11)));
        heap.mirror(5, handle(5), false).unwrap();
        STALE_TEST.set(None);
        assert!(!kept(4));
        assert!(!STALE_LATER.lock().unwrap().iter().any(|e| e.0 == std::ptr::from_ref(&heap) as usize));
    }

    /// The deferred stale slots of a heap whose submissions are done, and
    /// no other heap's.
    #[test]
    fn stale_slots_fall_due_by_heap_and_submission() {
        let mut later = vec![(1, 3, 30, 5), (1, 4, 40, 7), (2, 3, 31, 5)];
        assert_eq!(stale_due(&mut later, 1, 6), [(3, 30)]);
        assert_eq!(later, [(1, 4, 40, 7), (2, 3, 31, 5)]);
        assert!(stale_due(&mut later, 1, 6).is_empty());
        assert_eq!(stale_due(&mut later, 1, 7), [(4, 40)]);
        assert_eq!(later, [(2, 3, 31, 5)]);
    }

    #[test]
    fn an_old_unmapped_observation_preserves_a_replacement_view() {
        let Some(heap) = HostHeap::new(false, 8) else { return };
        let slot = 1;
        heap.queue_texture(slot, true, false, 10);
        let (_, observed) = heap.texture_slots_due(10, 1).0[0];

        // The replacement is a 64 x 64 BC7 texture with synthetic backing.
        static BYTES: [u8; 65536] = [0; 65536];
        let address = BYTES.as_ptr() as u64;
        let resource = [3, 0, 0, 0, 64, 0, 64, 0x1_0001, 98, 1, 0, 0x105, 0, 0];
        let mut srv = [0; 16];
        srv[..6].copy_from_slice(&[98, 4, 5768, 0, 0, 1]);
        (srv[10], srv[11]) = (address as u32, (address >> 32) as u32);
        let view = View::Placed(false, resource, srv);
        let [entry, generation] = super::super::words_for(view.clone());
        let title = [super::super::VIEW_TAG, entry, generation];
        let handle = title.as_ptr() as u64;
        assert_eq!(heap.mirror(slot, handle, false), Some(view.clone()));
        let (_, replacement) = heap.texture_slots_due(u64::MAX, 1).0[0];
        assert_ne!(observed, replacement);
        assert_eq!(heap.mirror(slot, handle, true), Some(view));
        assert_eq!(super::device_removed(), None);
        let key = u64::from(generation) << 32 | u64::from(entry);
        assert!(heap.kept_at(slot).is_some());
        assert!(!heap.texture_slot_gone(slot, observed));
        assert_eq!(heap.made.lock().unwrap()[slot as usize], key);
        assert_eq!(heap.texture_slots_due(u64::MAX, 1).0, [(slot, replacement)]);
        // A completed write is no longer queued for refresh.
        heap.texture_slot_done(slot, replacement, None);
        assert!(!heap.texture_slot_gone(slot, replacement));
        assert_eq!(heap.made.lock().unwrap()[slot as usize], key);
        heap.queue_texture(slot, true, false, 12);
        let (_, current) = heap.texture_slots_due(u64::MAX, 1).0[0];
        assert!(heap.texture_slot_gone(slot, current));
        assert_eq!(heap.made.lock().unwrap()[slot as usize], 0);
    }

    /// Memory the runtime maps under a slot's texture makes the slot due
    /// again from the next frame; a stale slot leaves the queue.
    #[test]
    fn remapped_memory_makes_its_texture_slots_due() {
        let Some(heap) = HostHeap::new(false, 8) else {
            return; // no host GPU
        };
        let slots = |at: u64| -> Vec<u32> { heap.texture_slots_due(at, 8).0.into_iter().map(|d| d.0).collect() };
        // 64x64 BC7, one mip, at an address no test and no title uses.
        let at = 0x7fee_0000_0000u64;
        let view = |address: u64, width: u32| {
            let mut desc = [0u32; 16];
            (desc[10], desc[11]) = (address as u32, (address >> 32) as u32);
            View::Placed(false, [3, 0, 0, 0, width, 0, 64, 0x1_0001, 98, 1, 0, 0x105, 0, 0], desc)
        };
        let len = crate::resource::placed_bytes(&[3, 0, 0, 0, 64, 0, 64, 0x1_0001, 98, 1, 0, 0x105, 0, 0]).unwrap();
        assert!(heap.note_buffer(1, Some(&view(at, 64)), false).is_empty());
        let (queued, write) = heap.texture_slots_due(u64::MAX, 8).0[0];
        assert_eq!(queued, 1);
        heap.texture_slot_done(1, write, None);
        assert_eq!(slots(u64::MAX), []);
        // Memory elsewhere: nothing. Under the texture: due from the next frame.
        remapped((at + 2 * len) as usize, 0x1000);
        remapped(at as usize, 0x1000);
        assert_eq!(heap.texture_slots_remapped(50), 1);
        assert_eq!(slots(50), []);
        assert_eq!(slots(51), [1]);
        // A wider texture written at the same address: slot 1 is stale.
        assert_eq!(heap.note_buffer(2, Some(&view(at, 128)), false), [1]);
        assert_eq!(slots(u64::MAX), [2]);
    }

    /// The table of strides marks a constant buffer view's slot, and a
    /// texture written over it after leaves no mark: a program that picks
    /// the slot then reads zeros, not the texture's descriptor as a
    /// constant buffer's.
    #[test]
    fn constant_buffer_slots_are_marked_in_the_stride_table() {
        let Some(heap) = HostHeap::new(false, 8) else {
            return; // no host GPU
        };
        if !victory_shader::emit::constant_marks() {
            return; // turned off for this run
        }
        let entry = |index: u32| -> u32 {
            let (_, _, data) = heap.strides.as_ref().expect("a view heap has a table of strides");
            // SAFETY: inside the mapping.
            unsafe { (*data as *const u32).add(index as usize).read_volatile() }
        };
        let _ = heap.note_buffer(3, Some(&View::Constant(0x1000, 256)), false);
        assert_eq!(entry(3), victory_shader::emit::SLOT_STRIDE_CONSTANT);
        // 64x64 BC7, one mip, at an address no test and no title uses.
        let at = 0x7fec_0000_0000u64;
        let mut desc = [0u32; 16];
        (desc[10], desc[11]) = (at as u32, (at >> 32) as u32);
        let texture = View::Placed(false, [3, 0, 0, 0, 64, 0, 64, 0x1_0001, 98, 1, 0, 0x105, 0, 0], desc);
        let _ = heap.note_buffer(3, Some(&texture), false);
        assert_eq!(entry(3), 0);
        let _ = heap.note_buffer(4, None, false);
        assert_eq!(entry(4), 0);
    }

    /// A slot a draw read through a table leaves the queue; the title's
    /// next write of it waits for its draw, and is due after the grace
    /// frames if none reads it.
    #[test]
    fn slots_read_through_tables_leave_the_texture_queue() {
        let Some(heap) = HostHeap::new(false, 8) else {
            return; // no host GPU
        };
        let frame = 100;
        let slots = |at: u64| -> Vec<u32> { heap.texture_slots_due(at, 8).0.into_iter().map(|d| d.0).collect() };
        let texture = View::Placed(false, [3, 0, 0, 0, 4, 0, 4, 0x1_0001, 28, 1, 0, 0x105, 0, 0], [0; 16]);
        assert!(is_texture_view(&texture));
        heap.queue_texture(2, true, false, frame);
        heap.queue_texture(3, true, false, frame);
        assert_eq!(slots(frame), [3, 2]);
        heap.tabled_in(3, true, frame);
        assert_eq!(slots(frame), [2]);
        // Made in place by the draw that read it: still out.
        heap.queue_texture(3, true, true, frame);
        assert_eq!(slots(frame), [2]);
        // Written again in the next frame: due once the grace frames have
        // passed without a draw that reads it.
        heap.queue_texture(3, true, false, frame + 1);
        assert_eq!(slots(frame + TABLED_GRACE), [2]);
        assert_eq!(slots(frame + 1 + TABLED_GRACE), [3, 2]);
        // The draw comes after all: out again.
        heap.tabled_in(3, true, frame + 1);
        assert_eq!(slots(u64::MAX), [2]);
        // A slot of a table no draw read stays queued; its next write waits.
        heap.tabled_in(2, false, frame + 1);
        assert_eq!(slots(frame + 1), [2]);
        heap.queue_texture(2, true, false, frame + 2);
        assert_eq!(slots(frame + 2), []);
        // Long after the last table, a write is due at once.
        heap.queue_texture(2, true, false, frame + 3 + TABLED_FRAMES);
        assert_eq!(slots(frame + 3 + TABLED_FRAMES), [2]);
    }

    #[test]
    fn dynamic_slots_are_held_until_released() {
        let Some(heap) = HostHeap::new(false, 4) else {
            return; // no host GPU
        };
        let a = heap.dynamic(heap.dynamic - 100).unwrap();
        let b = heap.dynamic(60).unwrap();
        assert_eq!((a, b), (4, 4 + heap.dynamic - 100));
        // 40 left at the end: 50 do not fit until something is given back.
        assert_eq!(heap.dynamic(50), None);
        heap.release(a, 50);
        assert_eq!(heap.dynamic(50), Some(4));
        assert_eq!(heap.dynamic(40), Some(4 + heap.dynamic - 40));
    }

    /// Placed unordered texture views (2D and 3D) mirror onto host unordered
    /// access textures when written into a shader-visible heap, reuse an
    /// existing target at the same address, and feed shader views placed at
    /// that address in another format through `texture::alias`.
    #[test]
    fn placed_unordered_texture_view_mirrors_and_aliases_to_shader_view() {
        let Some(host) = host() else {
            return; // no host GPU
        };
        let Some(heap) = HostHeap::new(false, 8) else {
            return;
        };

        // 1. 3D volume UAV (4x4x4 R32_UINT, resource flags = 0) mirrored via
        // `HostHeap::mirror(..., for_draw = false)` as a shader-visible heap
        // copy does.
        let vol_at = 0x7fed_1000_0000u64;
        let vol_res = [4, 0, 0, 0, 4, 0, 4, 0x1_0004, 42, 1, 0, 0, 0, 0];
        let mut vol_uav = [0u32; 16];
        (vol_uav[0], vol_uav[1], vol_uav[4]) = (42, 8, 4);
        (vol_uav[10], vol_uav[11]) = (vol_at as u32, (vol_at >> 32) as u32);
        let slot_mem = [0u64; 4];
        let slot_handle = slot_mem.as_ptr() as u64;
        super::super::create_placed_uav(std::ptr::null(), &vol_res, &vol_uav, slot_handle);
        assert!(heap.mirror(0, slot_handle, false).is_some());
        {
            let kept = heap.kept_at(0);
            let tex = kept.as_ref().expect("3D placed UAV must mirror immediately");
            // SAFETY: live D3D12 resource.
            let d = unsafe { tex.GetDesc() };
            assert_eq!(d.Dimension, D3D12_RESOURCE_DIMENSION_TEXTURE3D);
            assert_ne!(d.Flags.0 & D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS.0, 0);
        }
        // SAFETY: plain device query.
        assert!(unsafe { host.device.GetDeviceRemovedReason() }.is_ok());

        // Another placed UAV at the same address with compatible format reuses
        // the resource already recorded in `TARGETS_AT`.
        let Some(View::Unordered(first_vol, _, _)) =
            placed_texture(&View::Placed(true, vol_res, vol_uav))
        else {
            panic!("expected View::Unordered");
        };
        let mut explicit_flags = vol_res;
        explicit_flags[12] = 0x4;
        let Some(View::Unordered(second_vol, _, _)) =
            placed_texture(&View::Placed(true, explicit_flags, vol_uav))
        else {
            panic!("expected View::Unordered");
        };
        assert_eq!(first_vol, second_vol, "must reuse existing UAV texture at address");

        // 2. Cross-format aliasing: a 4x4 R9G9B9E5_SHAREDEXP (67) description
        // written through an R32_UINT (42) placed UAV and then read through an
        // R9G9B9E5_SHAREDEXP placed SRV (even if `ALLOW_UNORDERED_ACCESS` was
        // left on the SRV's resource description) copies the GPU texels across.
        let alias_at = 0x7fed_2000_0000u64;
        let rgb9e5_res = [3, 0, 0, 0, 4, 0, 4, 0x1_0001, 67, 1, 0, 0, 0x4, 0];
        let mut uav_desc = [0u32; 16];
        (uav_desc[0], uav_desc[1]) = (42, 4);
        (uav_desc[10], uav_desc[11]) = (alias_at as u32, (alias_at >> 32) as u32);
        let uav_placed = View::Placed(true, rgb9e5_res, uav_desc);
        let Some(View::Unordered(uav_obj, _, _)) = placed_texture(&uav_placed) else {
            panic!("expected placed UAV texture");
        };
        assert!(make(&uav_placed, heap.cpu(1)).flatten().is_some());

        let mut srv_words = [0u32; 16];
        (srv_words[0], srv_words[1], srv_words[2], srv_words[5]) =
            (67, 4, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, 1);
        (srv_words[10], srv_words[11]) = (alias_at as u32, (alias_at >> 32) as u32);
        let srv_placed = View::Placed(false, rgb9e5_res, srv_words);
        let Some(View::Shader(srv_obj, _)) = placed_texture(&srv_placed) else {
            panic!("expected placed SRV texture");
        };
        assert!(make(&srv_placed, heap.cpu(2)).flatten().is_some());
        assert_eq!(
            crate::resource::alias_of(srv_obj as crate::object::This),
            Some(uav_obj as crate::object::This),
            "placed SRV must find the placed UAV texture in TARGETS_AT"
        );

        // Upload a known R32_UINT pattern into `uav_obj` on the GPU, run
        // `texture::alias` into `srv_obj`, and read `srv_obj` back.
        let uav_tex = crate::resource::host_resource(uav_obj as crate::object::This).unwrap();
        let srv_tex = crate::resource::host_resource(srv_obj as crate::object::This).unwrap();
        let pattern = 0x7c40_8210u32;
        let make_buf = |heap_type, state| {
            let hp = D3D12_HEAP_PROPERTIES {
                Type: heap_type,
                ..Default::default()
            };
            let rd = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: 1024,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                ..Default::default()
            };
            let mut out: Option<ID3D12Resource> = None;
            // SAFETY: valid buffer description.
            unsafe {
                host.device
                    .CreateCommittedResource(&hp, D3D12_HEAP_FLAG_NONE, &rd, state, None, &mut out)
                    .unwrap();
            }
            out.unwrap()
        };
        let upload = make_buf(D3D12_HEAP_TYPE_UPLOAD, D3D12_RESOURCE_STATE_GENERIC_READ);
        let readback = make_buf(D3D12_HEAP_TYPE_READBACK, D3D12_RESOURCE_STATE_COPY_DEST);
        // SAFETY: 1024-byte upload buffer mapped on CPU.
        unsafe {
            let mut ptr = std::ptr::null_mut();
            upload.Map(0, None, Some(&mut ptr)).unwrap();
            std::slice::from_raw_parts_mut(ptr.cast::<u32>(), 256).fill(pattern);
            upload.Unmap(0, None);
        }

        let list = host.new_list().unwrap();
        crate::list::ensure_state(&list, uav_obj as crate::object::This, 0x400); // COPY_DEST
        let buf_loc = |buf: &ID3D12Resource, format| D3D12_TEXTURE_COPY_LOCATION {
            // SAFETY: borrowed pointer not released by the copy location.
            pResource: unsafe { std::mem::transmute_copy(buf) },
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                    Offset: 0,
                    Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                        Format: format,
                        Width: 4,
                        Height: 4,
                        Depth: 1,
                        RowPitch: 256,
                    },
                },
            },
        };
        let tex_loc = |tex: &ID3D12Resource| D3D12_TEXTURE_COPY_LOCATION {
            // SAFETY: borrowed pointer not released by the copy location.
            pResource: unsafe { std::mem::transmute_copy(tex) },
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                SubresourceIndex: 0,
            },
        };
        // SAFETY: open list and live resources in matching copy states.
        unsafe {
            list.list.CopyTextureRegion(
                &tex_loc(uav_tex),
                0,
                0,
                0,
                &buf_loc(&upload, DXGI_FORMAT_R32_UINT),
                None,
            );
        }
        crate::list::ensure_state(&list, uav_obj as crate::object::This, 0x8); // UNORDERED_ACCESS
        assert!(crate::texture::alias(
            &list,
            srv_obj as crate::object::This,
            uav_obj as crate::object::This
        ));
        crate::list::ensure_state(&list, srv_obj as crate::object::This, 0x800); // COPY_SOURCE
        // SAFETY: open list and `srv_tex` transitioned to `COPY_SOURCE`.
        unsafe {
            list.list.CopyTextureRegion(
                &buf_loc(&readback, DXGI_FORMAT_R9G9B9E5_SHAREDEXP),
                0,
                0,
                0,
                &tex_loc(srv_tex),
                None,
            );
        }
        list.close();
        host.execute(&[&list]);

        // SAFETY: GPU execution finished; readback holds 1024 bytes.
        let got = unsafe {
            let mut ptr = std::ptr::null_mut();
            readback.Map(0, None, Some(&mut ptr)).unwrap();
            let word = *ptr.cast::<u32>();
            readback.Unmap(0, None);
            word
        };
        assert_eq!(got, pattern);
        // SAFETY: plain device query.
        assert!(unsafe { host.device.GetDeviceRemovedReason() }.is_ok());
    }
}

#[cfg(test)]
mod record_print_tests {
    use super::record_print;

    /// Equal records print alike; a record that differs in any one byte,
    /// or in its length, prints otherwise.
    #[test]
    fn a_record_that_changed_prints_otherwise() {
        let record: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(37).wrapping_add(5)).collect();
        let print = record_print(&record);
        assert_eq!(print, record_print(&record.clone()));
        for at in 0..32 {
            for bit in 0..8 {
                let mut other = record.clone();
                other[at] ^= 1 << bit;
                assert_ne!(record_print(&other), print, "byte {at} bit {bit}");
            }
        }
        assert_ne!(record_print(&record[..16]), print);
        assert_ne!(record_print(&[0u8; 32]), record_print(&[0u8; 16]));
        // A record of zeros is told from "never looked at" by the caller's `| 1`.
        assert_ne!(record_print(&[0u8; 32]) | 1, 0);
    }
}

#[cfg(test)]
mod own_copy_tests {
    use super::own_bytes_if;

    /// A buffer view gets a copy of its own bytes only when it has no host
    /// view on its buffer's copy: off that copy's elements, and not a
    /// structured view that can be a raw view of the same bytes.
    #[test]
    fn a_view_off_its_copys_elements_gets_its_own_bytes() {
        let base = 0xd_9000_0000u64;
        // On an element: the buffer's copy (the two cases of every run's
        // log are the ones below).
        assert_eq!(own_bytes_if(true, 24, base, 3, 10, 12, 0, false), None);
        assert_eq!(own_bytes_if(true, 0, base, 0, 10, 12, 0, false), None);
        // A typed view of 12-byte elements, 5,222,240 bytes into its copy
        // (8 past an element): its own bytes, from its first element.
        assert_eq!(own_bytes_if(true, 5_222_240, base, 3, 10, 12, 0, false), Some((base + 36, 120)));
        // A structured view of 32-byte elements 8 bytes in: not on 16, so
        // no raw view of the copy either.
        assert_eq!(own_bytes_if(true, 8, base, 2, 4, 32, 32, true), Some((base + 64, 128)));
        // The same 16 bytes in: off its elements, but a raw view of the
        // copy's bytes does (as before).
        assert_eq!(own_bytes_if(true, 16, base, 2, 4, 32, 32, true), None);
        // No elements, no size, or the switch off: as before.
        assert_eq!(own_bytes_if(true, 8, base, 2, 0, 32, 32, true), None);
        assert_eq!(own_bytes_if(true, 8, base, 2, 4, 0, 0, false), None);
        assert_eq!(own_bytes_if(false, 8, base, 2, 4, 32, 32, true), None);
    }
}
