use maelstrom_core::identifiers::*;

#[test]
fn test_user_id_parse_valid() {
    let uid = UserId::parse("@alice:example.com").unwrap();
    assert_eq!(uid.localpart(), "alice");
    assert_eq!(uid.server_name(), "example.com");
    assert_eq!(uid.as_str(), "@alice:example.com");
}

#[test]
fn test_user_id_parse_invalid_no_sigil() {
    assert!(UserId::parse("alice:example.com").is_err());
}

#[test]
fn test_user_id_parse_invalid_no_colon() {
    assert!(UserId::parse("@alice").is_err());
}

#[test]
fn test_user_id_new() {
    let server = ServerName::new("example.com");
    let uid = UserId::new("bob", &server);
    assert_eq!(uid.as_str(), "@bob:example.com");
}

#[test]
fn test_room_id_parse() {
    let rid = RoomId::parse("!abc123:example.com").unwrap();
    assert_eq!(rid.as_str(), "!abc123:example.com");
    assert!(RoomId::parse("abc123:example.com").is_err());
}

#[test]
fn test_event_id_parse() {
    let eid = EventId::parse("$eventHash").unwrap();
    assert_eq!(eid.as_str(), "$eventHash");
    assert!(EventId::parse("eventHash").is_err());
}

#[test]
fn test_device_id_generate() {
    let d1 = DeviceId::generate();
    let d2 = DeviceId::generate();
    assert_ne!(d1.as_str(), d2.as_str());
    assert_eq!(d1.as_str().len(), 10);
}

#[test]
fn test_room_alias_parse() {
    let alias = RoomAlias::parse("#room:example.com").unwrap();
    assert_eq!(alias.as_str(), "#room:example.com");
    assert!(RoomAlias::parse("room:example.com").is_err());
}

#[test]
fn test_user_id_serde_roundtrip() {
    let uid = UserId::parse("@test:localhost").unwrap();
    let json = serde_json::to_string(&uid).unwrap();
    assert_eq!(json, r#""@test:localhost""#);

    let parsed: UserId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, uid);
}
