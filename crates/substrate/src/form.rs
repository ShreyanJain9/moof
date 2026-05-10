//! the universal heap kind.
//!
//! per `laws/substrate-laws.md` L1, every conceptually-allocated
//! value is a Form. concretely, a Form has the four-faces shape
//! (`docs/concepts/forms.md`):
//!
//! - **structure**: nothing here yet — phase A doesn't expose
//!   head/args separately. (parsed code-Forms are List Forms whose
//!   slots already carry head/tail; no extra fields needed at the
//!   substrate level.)
//! - **identity**: `proto` + `slots` + `handlers`.
//! - **history**: `meta`.
//! - **liveness**: not on every Form — vat-Forms get extra slots
//!   for mailbox/behavior at phase B.
//!
//! `slots`, `handlers`, and `meta` are `IndexMap`s for two reasons:
//!
//! 1. **insertion-order iteration is deterministic**, satisfying
//!    `laws/determinism-laws.md` D5. critical for replication.
//! 2. iteration is in the order users *added* keys, which is what
//!    they expect in inspectors and serializations.

use indexmap::IndexMap;

use crate::sym::SymId;
use crate::value::Value;

/// the four scopes a `FormId` can address. spec §5.
///
/// the top 2 bits of a 32-bit FormId encode the scope; the bottom 30
/// bits are the per-scope payload. vat-local is the only one with
/// real implementation in V0 — shared and far-ref panic until later
/// phases fill them in (V6 / V5 respectively).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Scope {
    /// `00…` — index into this vat's `Vec<Form>`.
    VatLocal,
    /// `01…` — index into the process-wide shared segment (V6).
    Shared,
    /// `10…` — index into this vat's far-ref table (V5).
    FarRef,
    /// `11…` — reserved for future use (NaN-boxed immediates,
    /// bigint pool, segmented heaps).
    Reserved,
}

/// the bit mask that selects the scope tag in a `FormId`'s u32.
pub const SCOPE_MASK: u32 = 0b11 << 30;
/// the bit mask that selects the payload in a `FormId`'s u32.
pub const PAYLOAD_MASK: u32 = !SCOPE_MASK;
/// the maximum payload value (exclusive). 2^30 ≈ 1.07 billion forms
/// per scope — vastly more than any reasonable vat needs.
pub const MAX_PAYLOAD: u32 = 1 << 30;

const TAG_VAT_LOCAL: u32 = 0b00 << 30;
const TAG_SHARED: u32 = 0b01 << 30;
const TAG_FAR_REF: u32 = 0b10 << 30;
const TAG_RESERVED: u32 = 0b11 << 30;

/// the heap-id of a Form. vat-local. stable within a vat
/// (`laws/substrate-laws.md` L11).
///
/// `FormId(0)` is reserved as a sentinel — never returned by
/// `Heap::alloc`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct FormId(pub u32);

impl FormId {
    pub const NONE: FormId = FormId(0);

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    /// the scope tag — which of the four spaces this id addresses.
    pub fn scope(self) -> Scope {
        match self.0 & SCOPE_MASK {
            TAG_VAT_LOCAL => Scope::VatLocal,
            TAG_SHARED => Scope::Shared,
            TAG_FAR_REF => Scope::FarRef,
            TAG_RESERVED => Scope::Reserved,
            _ => unreachable!("SCOPE_MASK selects exactly 2 bits"),
        }
    }

    /// the payload (per-scope index). bottom 30 bits.
    pub fn payload(self) -> u32 {
        self.0 & PAYLOAD_MASK
    }

    /// construct a vat-local FormId. payload must fit in 30 bits.
    pub fn vat_local(payload: u32) -> Self {
        assert!(payload < MAX_PAYLOAD, "vat-local payload exceeds 30-bit limit: {}", payload);
        FormId(TAG_VAT_LOCAL | payload)
    }

    /// construct a shared-segment FormId. payload must fit in 30 bits.
    pub fn shared(payload: u32) -> Self {
        assert!(payload < MAX_PAYLOAD, "shared-segment payload exceeds 30-bit limit: {}", payload);
        FormId(TAG_SHARED | payload)
    }

    /// construct a far-ref FormId. payload must fit in 30 bits.
    pub fn far_ref(payload: u32) -> Self {
        assert!(payload < MAX_PAYLOAD, "far-ref payload exceeds 30-bit limit: {}", payload);
        FormId(TAG_FAR_REF | payload)
    }
}

/// the universal heap kind.
///
/// every conceptually-allocated moof value is a Form. dispatch
/// walks `proto`. user data lives in `slots`. methods live in
/// `handlers`. provenance + annotations live in `meta`.
#[derive(Default)]
pub struct Form {
    /// the immediate delegation parent. `Value::Nil` for the root
    /// `Object` proto; `Value::Form(_)` for everything else.
    /// (`docs/concepts/objects-and-protos.md`.)
    pub proto: Value,

    /// named bindings. `IndexMap` so iteration order is insertion
    /// order — *deterministic* across replicas
    /// (`laws/determinism-laws.md` D5).
    pub slots: IndexMap<SymId, Value>,

    /// selector → method-Form (`Value::Form` of a method-shaped
    /// Form). protos populate this; instances rarely do.
    pub handlers: IndexMap<SymId, Value>,

    /// metadata: source-loc, doc, journal-id, type, etc.
    /// extensible by user code (`laws/reflection-contract.md` R7).
    pub meta: IndexMap<SymId, Value>,

    /// V2 — freezing. once `true`, `World::form_slot_set` /
    /// `form_handler_set` / `form_meta_set` raise `'frozen-form` on
    /// any write to this form's slots/handlers/meta. one-way
    /// (no thaw). transition itself is a turn-mutation: journals
    /// via the nursery, rolls back on abort.
    pub frozen: bool,
}

impl Form {
    /// build a Form with a given proto and otherwise empty.
    pub fn with_proto(proto: Value) -> Self {
        Form {
            proto,
            slots: IndexMap::new(),
            handlers: IndexMap::new(),
            meta: IndexMap::new(),
            frozen: false,
        }
    }

    /// look up a slot by name. returns `Value::Nil` if missing —
    /// callers that need to distinguish "missing" from "explicitly
    /// nil" use [`Form::slot_present`].
    pub fn slot(&self, name: SymId) -> Value {
        self.slots.get(&name).copied().unwrap_or(Value::Nil)
    }

    /// `true` if `name` is bound in this Form's slots.
    pub fn slot_present(&self, name: SymId) -> bool {
        self.slots.contains_key(&name)
    }

    /// look up a handler by selector. returns `None` if absent;
    /// callers walk the proto chain via `Heap::dispatch`.
    pub fn handler(&self, selector: SymId) -> Option<Value> {
        self.handlers.get(&selector).copied()
    }

    /// look up a meta entry. returns `Value::Nil` if missing.
    pub fn meta_at(&self, name: SymId) -> Value {
        self.meta.get(&name).copied().unwrap_or(Value::Nil)
    }
}
