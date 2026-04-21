// The canonical cons-pair as a `ForeignType` — this is the first
// of moof's core types to dogfood the foreign-registry pipeline
// (wave 5.1). Nothing else in moof treats pairs specially at the
// HeapObject level anymore; everything goes through the same
// registry-based dispatch that plugin types use.
//
// `car`/`cdr` are exposed as virtual slots so `p.car` / `p.cdr`
// work naturally. The proto is `PROTO_CONS`, and message dispatch
// (length, map:, fold:, …) goes through handlers on that proto
// exactly as before.

use crate::foreign::ForeignType;
use crate::value::Value;

#[derive(Clone, Debug)]
pub struct Pair {
    pub car: Value,
    pub cdr: Value,
}

impl ForeignType for Pair {
    fn type_name() -> &'static str { "moof.core.Pair" }
    fn prototype_name() -> &'static str { "Cons" }
    fn schema_version() -> u32 { 1 }

    fn trace(&self, visit: &mut dyn FnMut(Value)) {
        visit(self.car);
        visit(self.cdr);
    }

    fn clone_across(&self, copy: &mut dyn FnMut(Value) -> Value) -> Self {
        Pair { car: copy(self.car), cdr: copy(self.cdr) }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(16);
        b.extend_from_slice(&self.car.to_bits().to_le_bytes());
        b.extend_from_slice(&self.cdr.to_bits().to_le_bytes());
        b
    }

    fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != 16 { return Err(format!("Pair: expected 16 bytes, got {}", bytes.len())); }
        let mut a = [0u8; 8]; a.copy_from_slice(&bytes[0..8]);
        let mut b = [0u8; 8]; b.copy_from_slice(&bytes[8..16]);
        Ok(Pair {
            car: Value::from_bits(u64::from_le_bytes(a)),
            cdr: Value::from_bits(u64::from_le_bytes(b)),
        })
    }

    fn equal(&self, other: &Self) -> bool {
        self.car == other.car && self.cdr == other.cdr
    }

    fn describe(&self) -> String {
        // The heap isn't accessible here, so we fall back to the
        // debug-ish bit view. The real list-aware formatter lives
        // in heap::format and short-circuits before reaching this.
        format!("(0x{:016x} . 0x{:016x})", self.car.to_bits(), self.cdr.to_bits())
    }

    fn virtual_slot(&self, sym: u32) -> Option<Value> {
        let syms = PAIR_SYMS.load();
        if sym == syms.car { Some(self.car) }
        else if sym == syms.cdr { Some(self.cdr) }
        else { None }
    }

    fn virtual_slot_names(&self) -> Vec<u32> {
        let syms = PAIR_SYMS.load();
        vec![syms.car, syms.cdr]
    }
}

// Interned-symbol cache for `car` / `cdr`, populated at Heap::new()
// time (Pair is registered before any cons allocation). Atomic
// loads are single instructions and virtual_slot is called for
// every `.car` / `.cdr` on a Pair — worth the cache.
#[derive(Clone, Copy)]
pub(crate) struct PairSyms { pub car: u32, pub cdr: u32 }

pub(crate) struct AtomicPairSyms {
    car: std::sync::atomic::AtomicU32,
    cdr: std::sync::atomic::AtomicU32,
}
impl AtomicPairSyms {
    pub(crate) const fn new() -> Self {
        AtomicPairSyms {
            car: std::sync::atomic::AtomicU32::new(0),
            cdr: std::sync::atomic::AtomicU32::new(0),
        }
    }
    pub(crate) fn load(&self) -> PairSyms {
        use std::sync::atomic::Ordering::Relaxed;
        PairSyms { car: self.car.load(Relaxed), cdr: self.cdr.load(Relaxed) }
    }
    pub(crate) fn store(&self, s: PairSyms) {
        use std::sync::atomic::Ordering::Relaxed;
        self.car.store(s.car, Relaxed);
        self.cdr.store(s.cdr, Relaxed);
    }
}

pub(crate) static PAIR_SYMS: AtomicPairSyms = AtomicPairSyms::new();
