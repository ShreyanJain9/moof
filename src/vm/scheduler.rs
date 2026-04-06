/// Round-robin vat scheduler.
///
/// Owns all VatState instances. The VM borrows one at a time via swap_vat.
/// Single-threaded: only one vat runs at a time on the scheduler thread.

use std::time::Duration;
use super::exec::{VatState, VatStatus, Message, TurnResult, SpawnRequest, VM, VAT_TURN_FUEL};
use super::extension::MoofExtension;
use crate::runtime::value::{Value, HeapObject};

/// Unique vat identifier.
pub type VatId = u32;

pub struct Scheduler {
    /// All vats. Index = position, not necessarily VatId.
    vats: Vec<VatState>,
    /// Next vat id to assign.
    next_id: u32,
    /// Round-robin cursor: index into `vats` for next dispatch.
    cursor: usize,
    /// Default fuel for scheduled turns.
    pub default_fuel: u32,
    /// Registered extensions (polled each round for events).
    pub extensions: Vec<Box<dyn MoofExtension>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            vats: Vec::new(),
            next_id: 0,
            cursor: 0,
            default_fuel: VAT_TURN_FUEL,
            extensions: Vec::new(),
        }
    }

    /// Add an extension to the scheduler.
    pub fn add_extension(&mut self, ext: Box<dyn MoofExtension>) {
        self.extensions.push(ext);
    }

    /// Create a new vat and return its id. Does NOT set root_env — caller must do that.
    pub fn create_vat(&mut self) -> VatId {
        let id = self.next_id;
        self.next_id += 1;
        self.vats.push(VatState::new(id));
        id
    }

    /// Add an existing VatState (e.g. the one already on VM).
    pub fn add_vat(&mut self, vat: VatState) -> VatId {
        let id = vat.id;
        if id >= self.next_id {
            self.next_id = id + 1;
        }
        self.vats.push(vat);
        id
    }

    /// Number of vats.
    pub fn len(&self) -> usize {
        self.vats.len()
    }

    /// Find vat index by id.
    fn index_of(&self, id: VatId) -> Option<usize> {
        self.vats.iter().position(|v| v.id == id)
    }

    /// Borrow a vat by id (immutable).
    pub fn get(&self, id: VatId) -> Option<&VatState> {
        self.index_of(id).map(|i| &self.vats[i])
    }

    /// Borrow a vat by id (mutable).
    pub fn get_mut(&mut self, id: VatId) -> Option<&mut VatState> {
        self.index_of(id).map(move |i| &mut self.vats[i])
    }

    /// Swap the VM's active vat with one from the scheduler.
    /// Returns the previously active vat's id (now stored in scheduler).
    pub fn swap_in(&mut self, vm: &mut VM, vat_id: VatId) -> Option<VatId> {
        let idx = self.index_of(vat_id)?;
        let old_id = vm.vat.id;
        std::mem::swap(&mut vm.vat, &mut self.vats[idx]);
        // Now scheduler[idx] holds the old vat, vm.vat holds the requested one
        Some(old_id)
    }

    /// Park the VM's current vat back into the scheduler, and leave VM with a dummy.
    /// Returns the parked vat's id.
    pub fn park(&mut self, vm: &mut VM) -> VatId {
        let id = vm.vat.id;
        let mut parked = VatState::new(id);
        std::mem::swap(&mut vm.vat, &mut parked);
        // If this vat id already exists in the vec, replace it
        if let Some(idx) = self.index_of(id) {
            self.vats[idx] = parked;
        } else {
            self.vats.push(parked);
        }
        // Keep next_id above all known ids
        if id >= self.next_id {
            self.next_id = id + 1;
        }
        id
    }

    /// Run one round-robin pass: poll extensions for events, then
    /// deliver one message per ready vat. Returns the number of vats that ran.
    pub fn run_round(&mut self, vm: &mut VM) -> usize {
        let n = self.vats.len();
        if n == 0 && self.extensions.is_empty() {
            return 0;
        }

        // 1. Poll extensions for events and enqueue as messages
        for ext in &mut self.extensions {
            for event in ext.poll(Duration::ZERO) {
                if let Some(vat) = self.vats.iter_mut().find(|v| v.id == event.target_vat) {
                    vat.enqueue(Message {
                        receiver: event.receiver,
                        selector: event.selector,
                        args: event.args,
                        resolver: None,
                    });
                }
            }
        }

        let mut ran = 0;

        // 2. Collect (vat_id, message) pairs for vats that have pending messages
        let deliveries: Vec<(VatId, Message)> = self.vats.iter_mut()
            .filter(|v| v.status == VatStatus::Ready && v.has_messages())
            .map(|v| {
                let msg = v.dequeue().unwrap();
                (v.id, msg)
            })
            .collect();

        for (vat_id, msg) in deliveries {
            // Swap this vat into the VM
            if self.swap_in(vm, vat_id).is_none() {
                continue;
            }

            // Set fuel for the turn
            let fuel = self.default_fuel;

            // Deliver the message via message_send
            let result = Self::deliver_message(vm, &msg, fuel);

            // Handle result
            match &result {
                TurnResult::Error(e) => {
                    vm.vat.status = VatStatus::Suspended { error: e.clone() };
                }
                TurnResult::Completed(val) => {
                    // If there's a resolver (promise), resolve it
                    if let Some(resolver_id) = msg.resolver {
                        Self::resolve_promise(vm, resolver_id, *val);
                    }
                }
                TurnResult::Yielded => {
                    // Re-enqueue the message at the front so it resumes next turn
                    vm.vat.mailbox.push_front(msg);
                }
            }

            // Park the vat back
            self.park(vm);
            ran += 1;
        }

        ran
    }

    /// Deliver a single message by performing a message_send on the VM.
    fn deliver_message(vm: &mut VM, msg: &Message, fuel: u32) -> TurnResult {
        vm.vat.fuel = fuel;
        vm.vat.status = VatStatus::Running;

        let result = vm.message_send(
            Value::Object(msg.receiver),
            msg.selector,
            &msg.args,
        );

        match result {
            Ok(val) => {
                vm.vat.status = VatStatus::Ready;
                TurnResult::Completed(val)
            }
            Err(e) if e == "__yielded" => {
                vm.vat.status = VatStatus::Ready;
                TurnResult::Yielded
            }
            Err(e) => {
                vm.vat.status = VatStatus::Suspended { error: e.clone() };
                TurnResult::Error(e)
            }
        }
    }

    /// Resolve a promise object by setting its value and resolved slots.
    fn resolve_promise(vm: &mut VM, promise_id: u32, value: Value) {
        let val_sym = vm.heap.intern("value");
        let resolved_sym = vm.heap.intern("resolved");
        vm.heap.set_slot(promise_id, val_sym, value);
        vm.heap.set_slot(promise_id, resolved_sym, Value::True);
        // TODO: run waiters when we have the Promise protocol in moof
    }

    /// Enqueue a message for a specific vat (by id).
    /// Returns false if the vat doesn't exist.
    pub fn enqueue_message(&mut self, vat_id: VatId, msg: Message) -> bool {
        if let Some(vat) = self.get_mut(vat_id) {
            vat.enqueue(msg);
            true
        } else {
            false
        }
    }

    /// Drain the VM's spawn queue. For each SpawnRequest:
    /// 1. Create a new VatState with a fresh env (child of the spawning vat's root)
    /// 2. Enqueue a "call" message to invoke the function
    /// 3. Update the handle object with the assigned vat-id
    pub fn drain_spawns(&mut self, vm: &mut VM) {
        let requests: Vec<SpawnRequest> = vm.spawn_queue.drain(..).collect();
        let parent_root = vm.vat.root_env;

        for req in requests {
            let vat_id = self.create_vat();
            let vat = self.get_mut(vat_id).unwrap();

            // New vat gets a child environment of the spawner's root
            let new_env = if let Some(parent) = parent_root {
                vm.heap.alloc_env(Some(parent))
            } else {
                vm.heap.alloc_env(None)
            };
            vat.root_env = Some(new_env);

            // Enqueue a message to call the function (selector = "call:", no args)
            let call_sym = vm.sym_call;
            vat.enqueue(Message {
                receiver: match req.func {
                    Value::Object(id) => id,
                    _ => continue,
                },
                selector: call_sym,
                args: vec![],
                resolver: None,
            });

            // Update the handle object with the real vat-id and status
            let vat_id_sym = vm.heap.intern("vat-id");
            let status_sym = vm.heap.intern("status");
            let running_str = vm.heap.alloc_string("running");
            vm.heap.set_slot(req.handle_id, vat_id_sym, Value::Integer(vat_id as i64));
            vm.heap.set_slot(req.handle_id, status_sym, running_str);
        }
    }
}
