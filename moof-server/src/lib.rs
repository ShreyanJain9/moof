/// moof-server: a running objectspace.
///
/// The server wraps a Fabric and provides vat-based connections.
/// Every frontend — REPL, GUI, MCP agent, custom app — connects
/// as a vat. All interaction is message-passing through that vat.
///
/// The vat IS the connection. The vat IS the identity.
/// The vat IS the capability boundary.

use moof_fabric::*;
use moof_fabric::dispatch::HandlerInvoker;

/// A running objectspace.
pub struct Server {
    pub fabric: Fabric,
}

/// What a frontend can do when it connects.
#[derive(Debug, Clone)]
pub struct Capabilities {
    /// Objects this vat can access (faceted references).
    pub grants: Vec<(String, Value)>,
}

impl Default for Capabilities {
    fn default() -> Self {
        Capabilities { grants: Vec::new() }
    }
}

/// A frontend's connection to the server. Everything goes through here.
pub struct Connection {
    pub vat_id: VatId,
    /// Root environment for this vat (if a language shell is attached).
    pub root_env: Option<u32>,
}

impl Server {
    pub fn new() -> Self {
        Server {
            fabric: Fabric::new(),
        }
    }

    /// Register a handler invoker (language shells do this).
    pub fn register_invoker(&mut self, invoker: Box<dyn HandlerInvoker>) {
        self.fabric.register_invoker(invoker);
    }

    /// Connect a frontend. Creates a vat, grants capabilities.
    /// Returns a Connection handle.
    pub fn connect(&mut self, caps: Capabilities) -> Connection {
        let vat_id = self.fabric.create_vat();

        // Create a root object for this vat that holds its capabilities
        let root = self.fabric.create_object(Value::Nil);
        for (name, val) in &caps.grants {
            self.fabric.set_slot(root, name, *val);
        }

        // Set the vat's root
        if let Some(vat) = self.fabric.scheduler.get_mut(vat_id) {
            vat.root = Some(root);
        }

        Connection {
            vat_id,
            root_env: None,
        }
    }

    /// Run one scheduler tick: deliver pending messages across all vats.
    pub fn tick(&mut self) {
        self.fabric.tick();
    }

    /// Send a message synchronously (within the caller's turn).
    /// This is for in-process frontends that need immediate results.
    pub fn send(&mut self, receiver: Value, selector: u32, args: &[Value]) -> Result<Value, String> {
        self.fabric.send(receiver, selector, args)
    }

    /// Enqueue an async message for a vat.
    pub fn enqueue(&mut self, vat_id: VatId, msg: Message) -> bool {
        self.fabric.enqueue_message(vat_id, msg)
    }

    /// Access the fabric directly (for setup, bootstrapping).
    pub fn fabric(&mut self) -> &mut Fabric {
        &mut self.fabric
    }

    /// Intern a symbol.
    pub fn intern(&mut self, name: &str) -> u32 {
        self.fabric.intern(name)
    }
}

impl Connection {
    /// Set the root environment for this connection (language shells do this).
    pub fn set_root_env(&mut self, env: u32) {
        self.root_env = Some(env);
    }
}
