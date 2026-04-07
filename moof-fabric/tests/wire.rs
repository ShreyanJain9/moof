use moof_fabric::value::Value;
use moof_fabric::wire::*;

#[test]
fn test_request_roundtrip() {
    let req = Request::Send {
        receiver: 42,
        selector: "writeLine:".into(),
        args: vec![Value::Integer(99)],
    };
    let encoded = req.encode();
    let decoded = Request::decode(&encoded).unwrap();
    match decoded {
        Request::Send { receiver, selector, args } => {
            assert_eq!(receiver, 42);
            assert_eq!(selector, "writeLine:");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], Value::Integer(99));
        }
        _ => panic!("wrong request type"),
    }
}

#[test]
fn test_response_roundtrip() {
    let resp = Response::Connected {
        vat_id: 3,
        capabilities: vec![("Console".into(), 10), ("Clock".into(), 20)],
    };
    let encoded = resp.encode();
    let decoded = Response::decode(&encoded).unwrap();
    match decoded {
        Response::Connected { vat_id, capabilities } => {
            assert_eq!(vat_id, 3);
            assert_eq!(capabilities.len(), 2);
            assert_eq!(capabilities[0], ("Console".into(), 10));
            assert_eq!(capabilities[1], ("Clock".into(), 20));
        }
        _ => panic!("wrong response type"),
    }
}

#[test]
fn test_value_encoding() {
    let vals = vec![
        Value::Nil,
        Value::True,
        Value::False,
        Value::Integer(42),
        Value::Integer(-1),
        Value::Float(3.14),
        Value::Symbol(7),
        Value::Object(123),
    ];

    for val in &vals {
        let mut buf = Vec::new();
        encode_value(val, &mut buf);
        let mut pos = 0;
        let decoded = decode_value(&buf, &mut pos).unwrap();
        assert_eq!(decoded.as_value().unwrap(), *val);
    }
}

#[test]
fn test_framed_message() {
    let mut buf = Vec::new();
    let payload = b"hello fabric";
    write_message(&mut buf, payload).unwrap();

    let mut cursor = std::io::Cursor::new(buf);
    let read_back = read_message(&mut cursor).unwrap();
    assert_eq!(read_back, payload);
}

#[test]
fn test_connect_disconnect() {
    let req = Request::Connect { token: "dev-root".into() };
    let decoded = Request::decode(&req.encode()).unwrap();
    match decoded {
        Request::Connect { token } => assert_eq!(token, "dev-root"),
        _ => panic!("wrong"),
    }

    let req = Request::Disconnect;
    let decoded = Request::decode(&req.encode()).unwrap();
    assert!(matches!(decoded, Request::Disconnect));
}
