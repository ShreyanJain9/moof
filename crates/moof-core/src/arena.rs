// Object arena.
//
// Allocation, retrieval, and GC accounting for HeapObjects. The
// arena is the allocator; it doesn't know about symbols, prototypes,
// dispatch, or the foreign registry — those live on Heap and compose
// this.
//
// Allocation strategy: Vec<HeapObject> indexed by u32 id. GC reclaims
// slots by tombstoning (replacing with HeapObject::new_empty) and
// recording the id in free_list for reuse on the next alloc. This
// keeps ids stable — if something held a reference to an id across
// GC, the id doesn't get re-used to point at a different object
// (since GC only frees unreachable ones).

use crate::object::HeapObject;

pub struct Arena {
    objects: Vec<HeapObject>,
    /// Ids tombstoned by GC that can be reused on alloc.
    free_list: Vec<u32>,
    /// Counter since the last completed GC. Flips `gc_requested`
    /// when it crosses alloc_budget.
    allocs_since_gc: usize,
    /// Ratio-based budget: after each gc, set to live*2 (floored at
    /// MIN_GC_BUDGET). Adapts to working set.
    alloc_budget: usize,
    /// Set by alloc when the budget is crossed. The scheduler /
    /// REPL polls this at safepoints. NEVER run GC from a handler
    /// — frames would be live.
    pub gc_requested: bool,
}

impl Default for Arena {
    fn default() -> Self { Self::new() }
}

impl Arena {
    pub const MIN_GC_BUDGET: usize = 2048;
    pub const GC_GROWTH_FACTOR: usize = 2;

    pub fn new() -> Self {
        Arena {
            objects: Vec::new(),
            free_list: Vec::new(),
            allocs_since_gc: 0,
            alloc_budget: Self::MIN_GC_BUDGET,
            gc_requested: false,
        }
    }

    /// Allocate an object and return its u32 id. Uses a free-list
    /// slot if available; otherwise extends the backing vec. Flips
    /// `gc_requested` when the allocation budget is crossed.
    pub fn alloc(&mut self, obj: HeapObject) -> u32 {
        self.allocs_since_gc += 1;
        if self.allocs_since_gc >= self.alloc_budget {
            self.gc_requested = true;
        }
        if let Some(id) = self.free_list.pop() {
            self.objects[id as usize] = obj;
            id
        } else {
            let id = self.objects.len() as u32;
            self.objects.push(obj);
            id
        }
    }

    pub fn get(&self, id: u32) -> &HeapObject {
        &self.objects[id as usize]
    }

    pub fn get_mut(&mut self, id: u32) -> &mut HeapObject {
        &mut self.objects[id as usize]
    }

    /// Total capacity (including tombstoned slots). For "how many
    /// live objects are there" use `live_count`.
    pub fn len(&self) -> usize { self.objects.len() }

    pub fn is_empty(&self) -> bool { self.objects.is_empty() }

    /// Live objects = allocated slots minus free-list entries.
    pub fn live_count(&self) -> usize {
        self.objects.len() - self.free_list.len()
    }

    // ---- access for GC and image machinery ----

    pub fn objects(&self) -> &[HeapObject] { &self.objects }

    pub fn objects_mut_slice(&mut self) -> &mut [HeapObject] { &mut self.objects }

    pub fn free_list(&self) -> &[u32] { &self.free_list }

    pub fn push_free(&mut self, id: u32) { self.free_list.push(id); }

    /// Called at the END of a GC cycle. Resets counters and adjusts
    /// the allocation budget based on current live count.
    pub fn after_gc(&mut self, live: usize) {
        self.alloc_budget = (live * Self::GC_GROWTH_FACTOR).max(Self::MIN_GC_BUDGET);
        self.allocs_since_gc = 0;
        self.gc_requested = false;
    }

    /// Drop every object. Used by Heap::Drop to ensure foreign
    /// payloads drop before dylibs unload.
    pub fn clear(&mut self) {
        self.objects.clear();
        self.free_list.clear();
    }

    /// Replace the arena's object vec wholesale. Used by image
    /// load to rehydrate state. Also clears the free list (load
    /// assumes every slot is live; GC will re-populate later if
    /// needed).
    pub fn restore(&mut self, objects: Vec<HeapObject>) {
        self.objects = objects;
        self.free_list.clear();
        self.allocs_since_gc = 0;
        self.gc_requested = false;
    }
}
