/// Binary wire protocol for fabric operations.
///
/// Values are encoded as tagged bytes. Protocol messages are
/// Lists of Values. Language-neutral — any client that can
/// read/write this format can talk to the fabric.
///
/// Value encoding:
///   0x00           Nil
///   0x01           True
///   0x02           False
///   0x03 i64       Integer (big-endian)
///   0x04 f64       Float (big-endian bits)
///   0x05 u32       Symbol (server-local interned id)
///   0x06 u32       Object (server-local heap id)
///   0x07 u32 bytes String (length-prefixed UTF-8)
///   0x08 u32 vals  List (count-prefixed values)
///
/// A protocol message is: u32 (total byte length) followed by the encoded value.

use crate::value::Value;
use std::io::{Read, Write, Result as IoResult};

// ── Encoding ──

pub fn encode_value(val: &Value, buf: &mut Vec<u8>) {
    match val {
        Value::Nil => buf.push(0x00),
        Value::True => buf.push(0x01),
        Value::False => buf.push(0x02),
        Value::Integer(n) => {
            buf.push(0x03);
            buf.extend_from_slice(&n.to_be_bytes());
        }
        Value::Float(f) => {
            buf.push(0x04);
            buf.extend_from_slice(&f.to_bits().to_be_bytes());
        }
        Value::Symbol(id) => {
            buf.push(0x05);
            buf.extend_from_slice(&id.to_be_bytes());
        }
        Value::Object(id) => {
            buf.push(0x06);
            buf.extend_from_slice(&id.to_be_bytes());
        }
    }
}

/// Encode a string (not a Value — used for string fields in messages).
pub fn encode_string(s: &str, buf: &mut Vec<u8>) {
    buf.push(0x07);
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(bytes);
}

/// Encode a list of values.
pub fn encode_list(vals: &[Value], buf: &mut Vec<u8>) {
    buf.push(0x08);
    buf.extend_from_slice(&(vals.len() as u32).to_be_bytes());
    for v in vals {
        encode_value(v, buf);
    }
}

// ── Decoding ──

pub fn decode_value(data: &[u8], pos: &mut usize) -> Option<WireValue> {
    if *pos >= data.len() { return None; }
    let tag = data[*pos];
    *pos += 1;
    match tag {
        0x00 => Some(WireValue::Value(Value::Nil)),
        0x01 => Some(WireValue::Value(Value::True)),
        0x02 => Some(WireValue::Value(Value::False)),
        0x03 => {
            let n = i64::from_be_bytes(data[*pos..*pos+8].try_into().ok()?);
            *pos += 8;
            Some(WireValue::Value(Value::Integer(n)))
        }
        0x04 => {
            let bits = u64::from_be_bytes(data[*pos..*pos+8].try_into().ok()?);
            *pos += 8;
            Some(WireValue::Value(Value::Float(f64::from_bits(bits))))
        }
        0x05 => {
            let id = u32::from_be_bytes(data[*pos..*pos+4].try_into().ok()?);
            *pos += 4;
            Some(WireValue::Value(Value::Symbol(id)))
        }
        0x06 => {
            let id = u32::from_be_bytes(data[*pos..*pos+4].try_into().ok()?);
            *pos += 4;
            Some(WireValue::Value(Value::Object(id)))
        }
        0x07 => {
            let len = u32::from_be_bytes(data[*pos..*pos+4].try_into().ok()?) as usize;
            *pos += 4;
            let s = std::str::from_utf8(&data[*pos..*pos+len]).ok()?.to_string();
            *pos += len;
            Some(WireValue::String(s))
        }
        0x08 => {
            let count = u32::from_be_bytes(data[*pos..*pos+4].try_into().ok()?) as usize;
            *pos += 4;
            let mut vals = Vec::with_capacity(count);
            for _ in 0..count {
                vals.push(decode_value(data, pos)?);
            }
            Some(WireValue::List(vals))
        }
        _ => None,
    }
}

/// A decoded wire value — either a fabric Value or a String/List.
#[derive(Debug, Clone)]
pub enum WireValue {
    Value(Value),
    String(String),
    List(Vec<WireValue>),
}

impl WireValue {
    pub fn as_value(&self) -> Option<Value> {
        match self { WireValue::Value(v) => Some(*v), _ => None }
    }
    pub fn as_string(&self) -> Option<&str> {
        match self { WireValue::String(s) => Some(s), _ => None }
    }
    pub fn as_list(&self) -> Option<&[WireValue]> {
        match self { WireValue::List(v) => Some(v), _ => None }
    }
}

// ── Framed messages ──

/// Write a framed message: u32 length prefix + payload.
pub fn write_message<W: Write>(writer: &mut W, payload: &[u8]) -> IoResult<()> {
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

/// Read a framed message: u32 length prefix + payload.
pub fn read_message<R: Read>(reader: &mut R) -> IoResult<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

// ── Protocol operations ──

/// An argument in a Send request — either a fabric Value or a string
/// (which the server will allocate in the heap).
#[derive(Debug, Clone)]
pub enum WireArg {
    Val(Value),
    Str(String),
}

/// Fabric-level operations that a client can request.
#[derive(Debug)]
pub enum Request {
    /// Authenticate and get a vat.
    Connect { token: String },
    /// Send a message to an object.
    Send { receiver: u32, selector: String, args: Vec<WireArg> },
    /// Create a new object.
    Create { parent: Value },
    /// Read a slot.
    SlotGet { object: u32, slot: String },
    /// Write a slot.
    SlotSet { object: u32, slot: String, value: Value },
    /// Intern a symbol name.
    Intern { name: String },
    /// Disconnect.
    Disconnect,
}

/// Server responses.
#[derive(Debug)]
pub enum Response {
    /// Connection accepted.
    Connected { vat_id: u32, capabilities: Vec<(String, u32)> },
    /// Operation result.
    Ok(Value),
    /// Error.
    Error(String),
    /// Object created.
    Created(u32),
    /// Symbol interned.
    Interned(u32),
    /// Console output (forwarded from system vat).
    Output(String),
}

impl Request {
    /// Encode a request as bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Request::Connect { token } => {
                buf.push(0x10);
                encode_string(token, &mut buf);
            }
            Request::Send { receiver, selector, args } => {
                buf.push(0x11);
                buf.extend_from_slice(&receiver.to_be_bytes());
                encode_string(selector, &mut buf);
                buf.extend_from_slice(&(args.len() as u32).to_be_bytes());
                for arg in args {
                    match arg {
                        WireArg::Val(v) => encode_value(v, &mut buf),
                        WireArg::Str(s) => encode_string(s, &mut buf),
                    }
                }
            }
            Request::Create { parent } => {
                buf.push(0x12);
                encode_value(parent, &mut buf);
            }
            Request::SlotGet { object, slot } => {
                buf.push(0x13);
                buf.extend_from_slice(&object.to_be_bytes());
                encode_string(slot, &mut buf);
            }
            Request::SlotSet { object, slot, value } => {
                buf.push(0x14);
                buf.extend_from_slice(&object.to_be_bytes());
                encode_string(slot, &mut buf);
                encode_value(value, &mut buf);
            }
            Request::Intern { name } => {
                buf.push(0x15);
                encode_string(name, &mut buf);
            }
            Request::Disconnect => {
                buf.push(0x1F);
            }
        }
        buf
    }

    /// Decode a request from bytes.
    pub fn decode(data: &[u8]) -> Option<Request> {
        if data.is_empty() { return None; }
        let mut pos = 0;
        let op = data[pos]; pos += 1;
        match op {
            0x10 => {
                let token = decode_value(data, &mut pos)?.as_string()?.to_string();
                Some(Request::Connect { token })
            }
            0x11 => {
                let receiver = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?);
                pos += 4;
                let selector = decode_value(data, &mut pos)?.as_string()?.to_string();
                let arg_count = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?) as usize;
                pos += 4;
                let mut args = Vec::with_capacity(arg_count);
                for _ in 0..arg_count {
                    let wv = decode_value(data, &mut pos)?;
                    match wv {
                        WireValue::Value(v) => args.push(WireArg::Val(v)),
                        WireValue::String(s) => args.push(WireArg::Str(s)),
                        _ => return None,
                    }
                }
                Some(Request::Send { receiver, selector, args })
            }
            0x12 => {
                let parent = decode_value(data, &mut pos)?.as_value()?;
                Some(Request::Create { parent })
            }
            0x13 => {
                let object = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?);
                pos += 4;
                let slot = decode_value(data, &mut pos)?.as_string()?.to_string();
                Some(Request::SlotGet { object, slot })
            }
            0x14 => {
                let object = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?);
                pos += 4;
                let slot = decode_value(data, &mut pos)?.as_string()?.to_string();
                let value = decode_value(data, &mut pos)?.as_value()?;
                Some(Request::SlotSet { object, slot, value })
            }
            0x15 => {
                let name = decode_value(data, &mut pos)?.as_string()?.to_string();
                Some(Request::Intern { name })
            }
            0x1F => Some(Request::Disconnect),
            _ => None,
        }
    }
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Response::Connected { vat_id, capabilities } => {
                buf.push(0x20);
                buf.extend_from_slice(&vat_id.to_be_bytes());
                buf.extend_from_slice(&(capabilities.len() as u32).to_be_bytes());
                for (name, obj_id) in capabilities {
                    encode_string(name, &mut buf);
                    buf.extend_from_slice(&obj_id.to_be_bytes());
                }
            }
            Response::Ok(val) => {
                buf.push(0x21);
                encode_value(val, &mut buf);
            }
            Response::Error(msg) => {
                buf.push(0x22);
                encode_string(msg, &mut buf);
            }
            Response::Created(id) => {
                buf.push(0x23);
                buf.extend_from_slice(&id.to_be_bytes());
            }
            Response::Interned(id) => {
                buf.push(0x24);
                buf.extend_from_slice(&id.to_be_bytes());
            }
            Response::Output(text) => {
                buf.push(0x25);
                encode_string(text, &mut buf);
            }
        }
        buf
    }

    pub fn decode(data: &[u8]) -> Option<Response> {
        if data.is_empty() { return None; }
        let mut pos = 0;
        let op = data[pos]; pos += 1;
        match op {
            0x20 => {
                let vat_id = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?);
                pos += 4;
                let count = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?) as usize;
                pos += 4;
                let mut caps = Vec::with_capacity(count);
                for _ in 0..count {
                    let name = decode_value(data, &mut pos)?.as_string()?.to_string();
                    let obj_id = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?);
                    pos += 4;
                    caps.push((name, obj_id));
                }
                Some(Response::Connected { vat_id, capabilities: caps })
            }
            0x21 => {
                let val = decode_value(data, &mut pos)?.as_value()?;
                Some(Response::Ok(val))
            }
            0x22 => {
                let msg = decode_value(data, &mut pos)?.as_string()?.to_string();
                Some(Response::Error(msg))
            }
            0x23 => {
                let id = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?);
                Some(Response::Created(id))
            }
            0x24 => {
                let id = u32::from_be_bytes(data[pos..pos+4].try_into().ok()?);
                Some(Response::Interned(id))
            }
            0x25 => {
                let text = decode_value(data, &mut pos)?.as_string()?.to_string();
                Some(Response::Output(text))
            }
            _ => None,
        }
    }
}
