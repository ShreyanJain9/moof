/// moof-server: a running objectspace with vat-based isolation.
///
/// System vats (console, filesystem, clock) are always running.
/// Frontends authenticate and receive a user vat with references
/// to the system vats they're allowed to talk to.
///
/// A capability IS a reference. If you have it, you can send.
/// If you don't, the object doesn't exist in your world.

use moof_fabric::*;
use moof_fabric::dispatch::HandlerInvoker;
use std::collections::HashMap;

/// A running objectspace.
pub struct Server {
    pub fabric: Fabric,
    /// System vat handles
    pub system: SystemVats,
    /// Auth: token → role
    auth_tokens: HashMap<String, Role>,
}

/// The always-running system vats.
pub struct SystemVats {
    pub console: VatId,
    pub filesystem: VatId,
    pub clock: VatId,
    /// The interface objects that user vats get references to.
    /// These are objects IN the system vats that respond to messages.
    pub console_obj: u32,
    pub fs_obj: u32,
    pub clock_obj: u32,
}

/// What level of access a connection gets.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    /// Full access to everything. The REPL in dev mode.
    Root,
    /// Read + eval, writes go through review. AI agents.
    Agent,
    /// Sandboxed eval only. Untrusted code.
    Guest,
}

/// A frontend's connection. Holds the vat id and what it can reach.
pub struct Connection {
    pub vat_id: VatId,
    pub role: Role,
    pub root_env: Option<u32>,
    /// Which system vat references this connection holds.
    pub capabilities: Vec<(String, u32)>,
}

impl Server {
    /// Boot the server. Creates system vats.
    pub fn new() -> Self {
        let mut fabric = Fabric::new();

        // Create system vats
        let console_vat = fabric.create_vat();
        let fs_vat = fabric.create_vat();
        let clock_vat = fabric.create_vat();

        // Create the interface objects that live in the system vats.
        // User vats get references to these objects.
        // Sends to them are dispatched by the fabric — the system vat
        // processes them on its turn.
        let console_obj = fabric.create_object(Value::Nil);
        let fs_obj = fabric.create_object(Value::Nil);
        let clock_obj = fabric.create_object(Value::Nil);

        // Tag system objects for describe
        let tag_sym = fabric.intern("type-tag");
        let console_tag = Value::Symbol(fabric.intern("Console"));
        fabric.heap.slot_set(console_obj, tag_sym, console_tag);
        let fs_tag = Value::Symbol(fabric.intern("Filesystem"));
        fabric.heap.slot_set(fs_obj, tag_sym, fs_tag);
        let clock_tag = Value::Symbol(fabric.intern("Clock"));
        fabric.heap.slot_set(clock_obj, tag_sym, clock_tag);

        let system = SystemVats {
            console: console_vat,
            filesystem: fs_vat,
            clock: clock_vat,
            console_obj,
            fs_obj,
            clock_obj,
        };

        Server {
            fabric,
            system,
            auth_tokens: HashMap::new(),
        }
    }

    // ── Auth ──

    /// Register an auth token for a role.
    pub fn add_token(&mut self, token: &str, role: Role) {
        self.auth_tokens.insert(token.to_string(), role);
    }

    /// Authenticate with a token. Returns a connection with appropriate capabilities.
    pub fn authenticate(&mut self, token: &str) -> Result<Connection, String> {
        let role = self.auth_tokens.get(token)
            .cloned()
            .ok_or_else(|| "invalid token".to_string())?;
        Ok(self.connect(role))
    }

    /// Connect as root (dev mode, no auth needed).
    pub fn connect_root(&mut self) -> Connection {
        self.connect(Role::Root)
    }

    /// Connect with a specific role.
    pub fn connect(&mut self, role: Role) -> Connection {
        let vat_id = self.fabric.create_vat();

        // Grant capabilities based on role
        let capabilities = match &role {
            Role::Root => vec![
                ("console".into(), self.system.console_obj),
                ("fs".into(), self.system.fs_obj),
                ("clock".into(), self.system.clock_obj),
            ],
            Role::Agent => vec![
                ("clock".into(), self.system.clock_obj),
                // Agent gets read-only fs (TODO: facet wrapping)
                ("fs".into(), self.system.fs_obj),
            ],
            Role::Guest => vec![
                ("clock".into(), self.system.clock_obj),
                // Guest gets nothing else
            ],
        };

        Connection {
            vat_id,
            role,
            root_env: None,
            capabilities,
        }
    }

    // ── Lifecycle ──

    /// Run one scheduler tick.
    pub fn tick(&mut self) {
        self.fabric.tick();
    }

    /// Register a handler invoker (language shells do this).
    pub fn register_invoker(&mut self, invoker: Box<dyn HandlerInvoker>) {
        self.fabric.register_invoker(invoker);
    }

    /// Access the fabric directly (for setup, bootstrapping).
    pub fn fabric(&mut self) -> &mut Fabric {
        &mut self.fabric
    }

    /// Intern a symbol.
    pub fn intern(&mut self, name: &str) -> u32 {
        self.fabric.intern(name)
    }

    /// Send a message synchronously.
    pub fn send(&mut self, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        self.fabric.send(receiver, selector, args)
    }
}

impl Connection {
    pub fn set_root_env(&mut self, env: u32) {
        self.root_env = Some(env);
    }

    /// Bind this connection's capabilities into an environment.
    pub fn bind_capabilities(&self, fabric: &mut Fabric, env: u32) {
        for (name, obj_id) in &self.capabilities {
            let sym = fabric.intern(name);
            fabric.heap.env_define(env, sym, Value::Object(*obj_id));
        }
    }
}
