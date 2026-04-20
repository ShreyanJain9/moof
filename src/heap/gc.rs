// Mark-sweep tracing GC.
//
// Walks reachable objects from a set of roots, marks them live,
// then sweeps unmarked slots onto a freelist that `alloc` reuses.
// No compaction — existing object IDs stay stable, so every
// live reference (Values holding nursery IDs) remains valid
// across a GC.
//
// Roots come from three places:
//   - intrinsic to the heap: env, type_protos, outbox args,
//     ready_acts, spawn_queue payloads
//   - the VM's closure_descs constants (bytecode embeds Values)
//   - anything else the caller passes in via extra_roots (live
//     VM frame registers, if GC happens mid-execution)
//
// We only GC at safepoints — between REPL turns, when no
// frames are executing — so the "extra_roots" parameter is
// for forward-compat; today's callers pass &[].
//
// Native handlers (heap.natives) are NOT scanned. The convention
// is: native closures capture u32 symbol IDs only, never Values.
// capturing a Value in a native would be a GC bug.

use crate::object::HeapObject;
use crate::value::Value;
use crate::heap::{Heap, SpawnPayload};

/// Statistics from a GC pass. Useful for diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct GcStats {
    pub before: usize,   // total slots before sweep
    pub live: usize,     // live (marked) objects
    pub freed: usize,    // slots returned to the freelist this pass
    pub free_total: usize, // total freelist size after sweep
}

impl Heap {
    /// Run a mark-sweep GC. Returns the freshly freed count.
    ///
    /// extra_roots: Values from outside the heap (e.g. VM frame
    /// registers). Today's single-threaded REPL passes &[] — we
    /// only GC when the VM is idle.
    pub fn gc(&mut self, extra_roots: &[Value]) -> GcStats {
        let before = self.objects.len();
        let mut marked = vec![false; before];
        let mut worklist: Vec<u32> = Vec::with_capacity(before / 4);

        // ── seed from intrinsic roots ──

        // root environment
        mark_value_id(Value::nursery(self.env), &mut worklist, &mut marked);

        // type prototypes
        for &proto in &self.type_protos {
            mark_value_id(proto, &mut worklist, &mut marked);
        }

        // outbox — pending cross-vat sends
        for msg in &self.outbox {
            for &arg in &msg.args {
                mark_value_id(arg, &mut worklist, &mut marked);
            }
            mark_value_id(Value::nursery(msg.act_id), &mut worklist, &mut marked);
        }

        // ready acts
        for &aid in &self.ready_acts {
            if (aid as usize) < marked.len() && !marked[aid as usize] {
                marked[aid as usize] = true;
                worklist.push(aid);
            }
        }

        // spawn queue
        for req in &self.spawn_queue {
            mark_value_id(Value::nursery(req.act_id), &mut worklist, &mut marked);
            match &req.payload {
                SpawnPayload::Closure(v) => mark_value_id(*v, &mut worklist, &mut marked),
                SpawnPayload::ClosureWithArgs(v, args) => {
                    mark_value_id(*v, &mut worklist, &mut marked);
                    for &a in args { mark_value_id(a, &mut worklist, &mut marked); }
                }
                SpawnPayload::Source(_) => {}
            }
        }

        // extra roots (e.g. live VM registers — today unused)
        for &v in extra_roots {
            mark_value_id(v, &mut worklist, &mut marked);
        }

        // ── trace ──

        while let Some(id) = worklist.pop() {
            let idx = id as usize;
            if idx >= self.objects.len() { continue; }
            self.mark_children_of(idx, &mut worklist, &mut marked);
        }

        // ── sweep ──

        // collect existing free_list into a set so we don't double-free
        let free_set: std::collections::HashSet<u32> =
            self.free_list.iter().copied().collect();

        let mut newly_freed = 0usize;
        for i in 0..self.objects.len() {
            if !marked[i] && !free_set.contains(&(i as u32)) {
                // tombstone — overwrite so any stale access sees nothing.
                // parent=NIL prevents chain walking into this slot.
                self.objects[i] = HeapObject::new_empty(Value::NIL);
                self.free_list.push(i as u32);
                newly_freed += 1;
            }
        }

        let live = before - self.free_list.len();
        self.set_alloc_budget_from_live(live);
        self.gc_requested = false;
        GcStats {
            before,
            live,
            freed: newly_freed,
            free_total: self.free_list.len(),
        }
    }

    /// Walk children of object idx, marking + pushing to worklist.
    fn mark_children_of(&self, idx: usize, worklist: &mut Vec<u32>, marked: &mut [bool]) {
        match &self.objects[idx] {
            HeapObject::General { parent, slot_values, handlers, .. } => {
                mark_value_id(*parent, worklist, marked);
                for &v in slot_values { mark_value_id(v, worklist, marked); }
                for (_, v) in handlers { mark_value_id(*v, worklist, marked); }
            }
            HeapObject::Pair(car, cdr) => {
                mark_value_id(*car, worklist, marked);
                mark_value_id(*cdr, worklist, marked);
            }
            HeapObject::Text(_) | HeapObject::Buffer(_) => {
                // leaf — no outgoing refs
            }
            HeapObject::Table { seq, map } => {
                for &v in seq { mark_value_id(v, worklist, marked); }
                for (k, v) in map {
                    mark_value_id(*k, worklist, marked);
                    mark_value_id(*v, worklist, marked);
                }
            }
        }
    }
}

/// Mark a Value if it's a heap reference. Non-refs (primitives,
/// symbols, floats, booleans, nil) are no-ops.
fn mark_value_id(v: Value, worklist: &mut Vec<u32>, marked: &mut [bool]) {
    if let Some(id) = v.as_any_object() {
        let idx = id as usize;
        if idx < marked.len() && !marked[idx] {
            marked[idx] = true;
            worklist.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::Heap;

    #[test]
    fn gc_preserves_reachable() {
        let mut h = Heap::new();
        // bind a cons list (1 2 3) into the root environment
        let a = Value::integer(1);
        let b = Value::integer(2);
        let c = Value::integer(3);
        let tail = h.cons(c, Value::NIL);
        let mid = h.cons(b, tail);
        let head = h.cons(a, mid);
        let sym = h.intern("lst");
        h.env_def(sym, head);

        // also allocate some unreachable garbage
        let _junk1 = h.alloc_string("unreferenced");
        let _junk2 = h.cons(Value::integer(99), Value::NIL);

        let before = h.object_count();
        let stats = h.gc(&[]);

        // garbage freed
        assert!(stats.freed >= 2, "expected to free at least 2 slots, got {}", stats.freed);

        // the list is still reachable and valid
        let recovered = h.env_get(sym).unwrap();
        let items: Vec<i64> = h.list_to_vec(recovered).iter()
            .map(|v| v.as_integer().unwrap())
            .collect();
        assert_eq!(items, vec![1, 2, 3]);
        assert_eq!(stats.before, before);
    }

    #[test]
    fn alloc_reuses_freed_slots() {
        let mut h = Heap::new();
        // allocate some garbage, gc it
        for _ in 0..5 {
            h.alloc_string("garbage");
        }
        let size_before_gc = h.object_count();
        let stats = h.gc(&[]);
        assert!(stats.freed >= 5);

        // subsequent allocations should reuse freed slots — no growth
        let size_after_gc = h.object_count();
        for i in 0..3 {
            h.alloc_string(&format!("new{i}"));
        }
        let size_after_alloc = h.object_count();
        assert_eq!(size_after_alloc, size_after_gc,
            "heap grew {} → {} despite having {} free slots (size_before_gc={})",
            size_after_gc, size_after_alloc, stats.free_total, size_before_gc);
    }

    #[test]
    fn gc_preserves_closure_captures() {
        let mut h = Heap::new();
        // a Value in a closure's captures list must survive GC
        let captured_string = h.alloc_string("dont-collect-me");
        let captured_sym = h.intern("x");
        let closure = h.make_closure(0, 1, false, &[(captured_sym, captured_string)]);
        // root the closure in env
        let closure_sym = h.intern("f");
        h.env_def(closure_sym, closure);

        let _stats = h.gc(&[]);

        // the string should still be reachable via the closure
        let recovered_closure = h.env_get(closure_sym).unwrap();
        let caps = h.closure_captures(recovered_closure);
        let captured = caps.iter().find(|(n, _)| *n == captured_sym).map(|(_, v)| *v).unwrap();
        let s = h.get_string(captured.as_any_object().unwrap()).unwrap();
        assert_eq!(s, "dont-collect-me");
    }
}
