//! Vats: capability security domains with mailboxes.
//!
//! Every connection to the fabric gets its own vat.
//! Same-vat sends are synchronous. Cross-vat sends go through mailboxes.

use crate::value::Value;

/// A message in a vat's mailbox.
#[derive(Debug, Clone)]
pub struct Message {
    pub receiver: Value,
    pub selector: u32,
    pub args: Vec<Value>,
}

/// A vat: a security domain with a mailbox.
pub struct Vat {
    pub id: u32,
    pub capabilities: Vec<Value>,
    pub mailbox: Vec<Message>,
}

impl Vat {
    pub fn new(id: u32, capabilities: Vec<Value>) -> Self {
        Vat {
            id,
            capabilities,
            mailbox: Vec::new(),
        }
    }

    pub fn enqueue(&mut self, msg: Message) {
        self.mailbox.push(msg);
    }

    pub fn drain(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.mailbox)
    }
}

/// Round-robin scheduler across vats.
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

    pub fn create_vat(&mut self, capabilities: Vec<Value>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.vats.push(Vat::new(id, capabilities));
        id
    }

    pub fn vat_mut(&mut self, id: u32) -> Option<&mut Vat> {
        self.vats.iter_mut().find(|v| v.id == id)
    }

    pub fn vat(&self, id: u32) -> Option<&Vat> {
        self.vats.iter().find(|v| v.id == id)
    }
}
