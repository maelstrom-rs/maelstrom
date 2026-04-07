use maelstrom_core::error::{ErrorCode, MatrixError};

#[test]
fn test_error_code_serialization() {
    let code = ErrorCode::Forbidden;
    let json = serde_json::to_string(&code).unwrap();
    assert_eq!(json, r#""M_FORBIDDEN""#);
}

#[test]
fn test_error_code_deserialization() {
    let code: ErrorCode = serde_json::from_str(r#""M_UNKNOWN_TOKEN""#).unwrap();
    assert_eq!(code, ErrorCode::UnknownToken);
}

#[test]
fn test_matrix_error_serialization() {
    let err = MatrixError::forbidden("Access denied");
    let json = serde_json::to_string(&err).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(value["errcode"], "M_FORBIDDEN");
    assert_eq!(value["error"], "Access denied");
    // status field should NOT be in the serialized output
    assert!(value.get("status").is_none());
}

#[test]
fn test_error_display() {
    let err = MatrixError::not_found("Room not found");
    assert_eq!(err.to_string(), "M_NOT_FOUND: Room not found");
}
