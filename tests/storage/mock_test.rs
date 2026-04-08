use maelstrom_core::identifiers::{DeviceId, UserId};
use maelstrom_storage::mock::MockStorage;
use maelstrom_storage::traits::*;

fn test_user(localpart: &str) -> UserRecord {
    UserRecord {
        localpart: localpart.to_string(),
        password_hash: Some("hashed".to_string()),
        is_admin: false,
        is_guest: false,
        is_deactivated: false,
        created_at: chrono::Utc::now(),
    }
}

fn test_device(user_id: &str, device_id: &str) -> DeviceRecord {
    DeviceRecord {
        device_id: device_id.to_string(),
        user_id: user_id.to_string(),
        display_name: None,
        access_token: format!("token_{device_id}"),
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn test_create_and_get_user() {
    let store = MockStorage::new();
    let user = test_user("alice");

    store.create_user(&user).await.unwrap();

    let fetched = store.get_user("alice").await.unwrap();
    assert_eq!(fetched.localpart, "alice");
}

#[tokio::test]
async fn test_create_duplicate_user_fails() {
    let store = MockStorage::new();
    let user = test_user("alice");

    store.create_user(&user).await.unwrap();
    let result = store.create_user(&user).await;

    assert!(matches!(result, Err(StorageError::Duplicate(_))));
}

#[tokio::test]
async fn test_user_exists() {
    let store = MockStorage::new();
    assert!(!store.user_exists("alice").await.unwrap());

    store.create_user(&test_user("alice")).await.unwrap();
    assert!(store.user_exists("alice").await.unwrap());
}

#[tokio::test]
async fn test_get_nonexistent_user_fails() {
    let store = MockStorage::new();
    let result = store.get_user("nobody").await;
    assert!(matches!(result, Err(StorageError::NotFound)));
}

#[tokio::test]
async fn test_profile_created_with_user() {
    let store = MockStorage::new();
    store.create_user(&test_user("alice")).await.unwrap();

    let profile = store.get_profile("alice").await.unwrap();
    assert!(profile.display_name.is_none());
    assert!(profile.avatar_url.is_none());
}

#[tokio::test]
async fn test_set_display_name() {
    let store = MockStorage::new();
    store.create_user(&test_user("alice")).await.unwrap();

    store
        .set_display_name("alice", Some("Alice"))
        .await
        .unwrap();

    let profile = store.get_profile("alice").await.unwrap();
    assert_eq!(profile.display_name.as_deref(), Some("Alice"));
}

#[tokio::test]
async fn test_create_and_get_device() {
    let store = MockStorage::new();
    let device = test_device("@alice:localhost", "DEV001");

    store.create_device(&device).await.unwrap();

    let uid = UserId::parse("@alice:localhost").unwrap();
    let did = DeviceId::new("DEV001");
    let fetched = store.get_device(&uid, &did).await.unwrap();

    assert_eq!(fetched.device_id, "DEV001");
    assert_eq!(fetched.access_token, "token_DEV001");
}

#[tokio::test]
async fn test_get_device_by_token() {
    let store = MockStorage::new();
    store
        .create_device(&test_device("@alice:localhost", "DEV001"))
        .await
        .unwrap();

    let fetched = store.get_device_by_token("token_DEV001").await.unwrap();
    assert_eq!(fetched.device_id, "DEV001");
}

#[tokio::test]
async fn test_list_devices() {
    let store = MockStorage::new();
    store
        .create_device(&test_device("@alice:localhost", "DEV001"))
        .await
        .unwrap();
    store
        .create_device(&test_device("@alice:localhost", "DEV002"))
        .await
        .unwrap();
    store
        .create_device(&test_device("@bob:localhost", "DEV003"))
        .await
        .unwrap();

    let uid = UserId::parse("@alice:localhost").unwrap();
    let devices = store.list_devices(&uid).await.unwrap();
    assert_eq!(devices.len(), 2);
}

#[tokio::test]
async fn test_remove_device() {
    let store = MockStorage::new();
    store
        .create_device(&test_device("@alice:localhost", "DEV001"))
        .await
        .unwrap();

    let uid = UserId::parse("@alice:localhost").unwrap();
    let did = DeviceId::new("DEV001");
    store.remove_device(&uid, &did).await.unwrap();

    let result = store.get_device(&uid, &did).await;
    assert!(matches!(result, Err(StorageError::NotFound)));
}

#[tokio::test]
async fn test_remove_all_devices() {
    let store = MockStorage::new();
    store
        .create_device(&test_device("@alice:localhost", "DEV001"))
        .await
        .unwrap();
    store
        .create_device(&test_device("@alice:localhost", "DEV002"))
        .await
        .unwrap();

    let uid = UserId::parse("@alice:localhost").unwrap();
    store.remove_all_devices(&uid).await.unwrap();

    let devices = store.list_devices(&uid).await.unwrap();
    assert!(devices.is_empty());
}

#[tokio::test]
async fn test_health_check() {
    let store = MockStorage::new();
    assert!(store.is_healthy().await);

    store.set_healthy(false);
    assert!(!store.is_healthy().await);
}
