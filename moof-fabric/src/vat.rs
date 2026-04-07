/// Vats: the concurrency and capability boundary.
///
/// A vat is a security domain with its own mailbox. Every frontend,
/// every I/O channel, every agent is a vat. Pure computation happens
/// inside a vat turn. Effects are message sends to capability objects.
///
/// The vat IS the monad.

use std::collections::VecDeque;
use crate::value::Value;

pub type VatId = u32;

/// A message queued for delivery.
#[derive(Debug, Clone)]
pub struct Message {
    pub receiver: u32,
    pub selector: u32,
    pub args: Vec<Value>,
    pub resolver: Option<u32>,
}

/// Per-vat state.
pub struct Vat {
    pub id: VatId,
    pub mailbox: VecDeque<Message>,
    pub status: VatStatus,
    /// Fuel remaining per turn. 0 = unlimited.
    pub fuel: u32,
    /// Root object for this vat (holds its capabilities).
    pub root: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VatStatus {
    Ready,
    Running,
    Suspended { error: String },
}

pub const DEFAULT_FUEL: u32 = 0;
pub const TURN_FUEL: u32 = 10_000;

impl Vat {
    pub fn new(id: VatId) -> Self {
        Vat {
            id,
            mailbox: VecDeque::new(),
            status: VatStatus::Ready,
            fuel: DEFAULT_FUEL,
            root: None,
        }
    }

    pub fn enqueue(&mut self, msg: Message) {
        self.mailbox.push_back(msg);
    }

    pub fn dequeue(&mut self) -> Option<Message> {
        self.mailbox.pop_front()
    }

    pub fn has_messages(&self) -> bool {
        !self.mailbox.is_empty()
    }
}

/// The scheduler: owns all vats, round-robin dispatch.
pub struct Scheduler {
    vats: Vec<Vat>,
    next_id: u32,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            vats: Vec::new(),
            next_id: 0,
        }
    }

    pub fn create_vat(&mut self) -> VatId {
        let id = self.next_id;
        self.next_id += 1;
        self.vats.push(Vat::new(id));
        id
    }

    pub fn get(&self, id: VatId) -> Option<&Vat> {
        self.vats.iter().find(|v| v.id == id)
    }

    pub fn get_mut(&mut self, id: VatId) -> Option<&mut Vat> {
        self.vats.iter_mut().find(|v| v.id == id)
    }

    pub fn enqueue(&mut self, vat_id: VatId, msg: Message) -> bool {
        if let Some(vat) = self.get_mut(vat_id) {
            vat.enqueue(msg);
            true
        } else {
            false
        }
    }

    /// Collect messages from all ready vats. Returns (vat_id, message) pairs.
    pub fn collect_ready_messages(&mut self) -> Vec<(VatId, Message)> {
        self.vats.iter_mut()
            .filter(|v| v.status == VatStatus::Ready && v.has_messages())
            .map(|v| {
                let msg = v.dequeue().unwrap();
                (v.id, msg)
            })
            .collect()
    }

    pub fn vat_count(&self) -> usize {
        self.vats.len()
    }

    pub fn set_status(&mut self, id: VatId, status: VatStatus) {
        if let Some(vat) = self.get_mut(id) {
            vat.status = status;
        }
    }
}
