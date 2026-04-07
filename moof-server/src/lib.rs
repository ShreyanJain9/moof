/// moof-server: a running objectspace with vat-based isolation.
///
/// System vats (Console, Filesystem, Clock) are always running.
/// External interfaces (FFI, HTTP, databases) are virtual vats too.
///
/// Frontends connect and receive specific object references.
/// A capability IS a reference. No roles. No permissions system.
/// You have the reference or you don't.

pub mod extension;
pub mod io;

use moof_fabric::*;
use moof_fabric::dispatch::HandlerInvoker;
use std::collections::HashMap;

/// A running objectspace.
pub struct Server {
    pub fabric: Fabric,
    pub system: SystemVats,
    /// Auth: token → list of capability names to grant.
    /// The server operator configures this.
    auth_tokens: HashMap<String, Vec<String>>,
}

/// Always-running system vats and their interface objects.
pub struct SystemVats {
    pub console_vat: VatId,
    pub filesystem_vat: VatId,
    pub clock_vat: VatId,
    /// Interface objects (what user vats get references to)
    pub Console: u32,
    pub Filesystem: u32,
    pub Clock: u32,
    /// All system objects by name (for capability grants)
    pub by_name: HashMap<String, u32>,
}

/// A frontend's connection.
pub struct Connection {
    pub vat_id: VatId,
    pub root_env: Option<u32>,
    /// Which object references this connection holds.
    pub capabilities: Vec<(String, u32)>,
}

impl Server {
    /// Boot the server. Creates system vats.
    pub fn new() -> Self {
        let mut fabric = Fabric::new();

        let console_vat = fabric.create_vat();
        let filesystem_vat = fabric.create_vat();
        let clock_vat = fabric.create_vat();

        let console_obj = fabric.create_object(Value::Nil);
        let fs_obj = fabric.create_object(Value::Nil);
        let clock_obj = fabric.create_object(Value::Nil);

        // Tag system objects
        let tag_sym = fabric.intern("type-tag");
        let console_tag = fabric.intern("Console");
        let fs_tag = fabric.intern("Filesystem");
        let clock_tag = fabric.intern("Clock");
        fabric.heap.slot_set(console_obj, tag_sym, Value::Symbol(console_tag));
        fabric.heap.slot_set(fs_obj, tag_sym, Value::Symbol(fs_tag));
        fabric.heap.slot_set(clock_obj, tag_sym, Value::Symbol(clock_tag));

        let mut by_name = HashMap::new();
        by_name.insert("Console".into(), console_obj);
        by_name.insert("Filesystem".into(), fs_obj);
        by_name.insert("Clock".into(), clock_obj);

        Server {
            fabric,
            system: SystemVats {
                console_vat, filesystem_vat, clock_vat,
                Console: console_obj, Filesystem: fs_obj, Clock: clock_obj,
                by_name,
            },
            auth_tokens: HashMap::new(),
        }
    }

    // ── Virtual vats ──

    /// Register a new virtual vat (external interface).
    /// Returns the interface object id. Handlers are added by the caller.
    pub fn add_virtual_vat(&mut self, name: &str) -> u32 {
        let vat_id = self.fabric.create_vat();
        let obj = self.fabric.create_object(Value::Nil);
        let tag_sym = self.fabric.intern("type-tag");
        let tag_val = Value::Symbol(self.fabric.intern(name));
        self.fabric.heap.slot_set(obj, tag_sym, tag_val);
        self.system.by_name.insert(name.to_string(), obj);
        obj
    }

    // ── Auth ──

    /// Register a token that grants specific capability names.
    pub fn add_token(&mut self, token: &str, capability_names: Vec<String>) {
        self.auth_tokens.insert(token.to_string(), capability_names);
    }

    /// Authenticate with a token. Returns a connection with the granted references.
    pub fn authenticate(&mut self, token: &str) -> Result<Connection, String> {
        let cap_names = self.auth_tokens.get(token)
            .cloned()
            .ok_or_else(|| "invalid token".to_string())?;
        self.connect_with(&cap_names)
    }

    /// Connect with specific capability names (looked up in system.by_name).
    pub fn connect_with(&mut self, capability_names: &[String]) -> Result<Connection, String> {
        let vat_id = self.fabric.create_vat();
        let mut capabilities = Vec::new();
        for name in capability_names {
            if let Some(&obj_id) = self.system.by_name.get(name.as_str()) {
                capabilities.push((name.clone(), obj_id));
            }
        }
        Ok(Connection { vat_id, root_env: None, capabilities })
    }

    /// Connect with ALL system capabilities (dev mode).
    pub fn connect_all(&mut self) -> Connection {
        let vat_id = self.fabric.create_vat();
        let capabilities: Vec<(String, u32)> = self.system.by_name.iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        Connection { vat_id, root_env: None, capabilities }
    }

    // ── Lifecycle ──

    pub fn tick(&mut self) { self.fabric.tick(); }
    pub fn register_invoker(&mut self, invoker: Box<dyn HandlerInvoker>) { self.fabric.register_invoker(invoker); }
    pub fn fabric(&mut self) -> &mut Fabric { &mut self.fabric }
    pub fn intern(&mut self, name: &str) -> u32 { self.fabric.intern(name) }
}

impl Connection {
    pub fn set_root_env(&mut self, env: u32) { self.root_env = Some(env); }

    /// Bind this connection's capabilities into an environment.
    pub fn bind_capabilities(&self, fabric: &mut Fabric, env: u32) {
        for (name, obj_id) in &self.capabilities {
            let sym = fabric.intern(name);
            fabric.heap.env_define(env, sym, Value::Object(*obj_id));
        }
    }
}
