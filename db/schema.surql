-- =============================================================
-- Maelstrom SurrealDB Schema
-- =============================================================
-- This file is the single source of truth for the database schema.
-- It is executed on every startup and is idempotent (IF NOT EXISTS).
--
-- Maelstrom leverages SurrealDB's graph model throughout:
-- - Event DAG: event --(event_edge)--> event (prev_events, auth_events)
-- - Membership: user --(member_of)--> room
-- - Relations: event --(relates_to)--> event (threads, reactions, edits)
-- =============================================================

-- =============================================================
-- Users
-- =============================================================
DEFINE TABLE IF NOT EXISTS user SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS localpart       ON TABLE user TYPE string;
DEFINE FIELD IF NOT EXISTS password_hash   ON TABLE user TYPE option<string>;
DEFINE FIELD IF NOT EXISTS is_admin        ON TABLE user TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS is_guest        ON TABLE user TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS is_deactivated  ON TABLE user TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS created_at      ON TABLE user TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_user_localpart ON TABLE user FIELDS localpart UNIQUE;

-- =============================================================
-- User Profiles
-- =============================================================
DEFINE TABLE IF NOT EXISTS profile SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user         ON TABLE profile TYPE record<user>;
DEFINE FIELD IF NOT EXISTS display_name ON TABLE profile TYPE option<string>;
DEFINE FIELD IF NOT EXISTS avatar_url   ON TABLE profile TYPE option<string>;

DEFINE INDEX IF NOT EXISTS idx_profile_user ON TABLE profile FIELDS user UNIQUE;

-- =============================================================
-- Devices
-- =============================================================
DEFINE TABLE IF NOT EXISTS device SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device_id     ON TABLE device TYPE string;
DEFINE FIELD IF NOT EXISTS user          ON TABLE device TYPE record<user>;
DEFINE FIELD IF NOT EXISTS display_name  ON TABLE device TYPE option<string>;
DEFINE FIELD IF NOT EXISTS access_token  ON TABLE device TYPE string;
DEFINE FIELD IF NOT EXISTS created_at    ON TABLE device TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_device_access_token ON TABLE device FIELDS access_token UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_device_user_device  ON TABLE device FIELDS user, device_id UNIQUE;

-- =============================================================
-- Rooms
-- =============================================================
DEFINE TABLE IF NOT EXISTS room SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS room_id      ON TABLE room TYPE string;
DEFINE FIELD IF NOT EXISTS version      ON TABLE room TYPE string DEFAULT "11";
DEFINE FIELD IF NOT EXISTS creator      ON TABLE room TYPE string;
DEFINE FIELD IF NOT EXISTS is_direct    ON TABLE room TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS created_at   ON TABLE room TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS visibility   ON TABLE room TYPE string DEFAULT "private";

DEFINE INDEX IF NOT EXISTS idx_room_room_id ON TABLE room FIELDS room_id UNIQUE;

-- =============================================================
-- Room Aliases
-- =============================================================
DEFINE TABLE IF NOT EXISTS room_alias SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS alias   ON TABLE room_alias TYPE string;
DEFINE FIELD IF NOT EXISTS room_id ON TABLE room_alias TYPE string;
DEFINE FIELD IF NOT EXISTS creator ON TABLE room_alias TYPE string;
DEFINE INDEX IF NOT EXISTS idx_room_alias_alias ON TABLE room_alias FIELDS alias UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_room_alias_room  ON TABLE room_alias FIELDS room_id;

-- =============================================================
-- Membership: user --(member_of)--> room (graph relation)
-- =============================================================
DEFINE TABLE IF NOT EXISTS member_of TYPE RELATION IN user OUT room SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS membership ON TABLE member_of TYPE string;
DEFINE FIELD IF NOT EXISTS user_id    ON TABLE member_of TYPE string;  -- denormalized for query convenience
DEFINE FIELD IF NOT EXISTS room_id    ON TABLE member_of TYPE string;  -- denormalized for query convenience
DEFINE FIELD IF NOT EXISTS updated_at ON TABLE member_of TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_member_of_in_out      ON TABLE member_of FIELDS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_member_of_out         ON TABLE member_of FIELDS out;
DEFINE INDEX IF NOT EXISTS idx_member_of_in          ON TABLE member_of FIELDS in;
DEFINE INDEX IF NOT EXISTS idx_member_of_membership  ON TABLE member_of FIELDS membership;

-- =============================================================
-- Events (PDUs)
-- =============================================================
DEFINE TABLE IF NOT EXISTS event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS event_id         ON TABLE event TYPE string;
DEFINE FIELD IF NOT EXISTS room_id          ON TABLE event TYPE string;
DEFINE FIELD IF NOT EXISTS sender           ON TABLE event TYPE string;
DEFINE FIELD IF NOT EXISTS event_type       ON TABLE event TYPE string;
DEFINE FIELD IF NOT EXISTS state_key        ON TABLE event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS content          ON TABLE event TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS origin_server_ts ON TABLE event TYPE int;
DEFINE FIELD IF NOT EXISTS unsigned_data    ON TABLE event TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS stream_position  ON TABLE event TYPE int;
DEFINE FIELD IF NOT EXISTS depth            ON TABLE event TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS origin           ON TABLE event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS auth_events      ON TABLE event TYPE option<array>;
DEFINE FIELD IF NOT EXISTS auth_events.*    ON TABLE event TYPE string;
DEFINE FIELD IF NOT EXISTS prev_events      ON TABLE event TYPE option<array>;
DEFINE FIELD IF NOT EXISTS prev_events.*    ON TABLE event TYPE string;
DEFINE FIELD IF NOT EXISTS hashes           ON TABLE event TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS signatures       ON TABLE event TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS created_at       ON TABLE event TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_event_event_id    ON TABLE event FIELDS event_id UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_event_room_stream ON TABLE event FIELDS room_id, stream_position;
DEFINE INDEX IF NOT EXISTS idx_event_stream      ON TABLE event FIELDS stream_position;

-- Compound index for state_event_at queries
DEFINE INDEX IF NOT EXISTS idx_event_room_type_state ON TABLE event FIELDS room_id, event_type, state_key, stream_position;

-- Full-text search on message content
DEFINE ANALYZER IF NOT EXISTS maelstrom_text TOKENIZERS blank, class FILTERS lowercase, snowball(english);
DEFINE INDEX IF NOT EXISTS idx_event_content_body ON TABLE event FIELDS content.body FULLTEXT ANALYZER maelstrom_text BM25(1.2, 0.75);

-- =============================================================
-- Room State: current state map per room
-- Maps (room_id, event_type, state_key) -> event_id
-- =============================================================
DEFINE TABLE IF NOT EXISTS room_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS room_id    ON TABLE room_state TYPE string;
DEFINE FIELD IF NOT EXISTS event_type ON TABLE room_state TYPE string;
DEFINE FIELD IF NOT EXISTS state_key  ON TABLE room_state TYPE string;
DEFINE FIELD IF NOT EXISTS event_id   ON TABLE room_state TYPE string;

DEFINE INDEX IF NOT EXISTS idx_room_state_unique ON TABLE room_state FIELDS room_id, event_type, state_key UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_room_state_room   ON TABLE room_state FIELDS room_id;

-- =============================================================
-- Stream Counter: monotonic position for sync ordering
-- =============================================================
DEFINE TABLE IF NOT EXISTS stream_counter SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS position ON TABLE stream_counter TYPE int DEFAULT 0;

INSERT INTO stream_counter { id: stream_counter:global, position: 0 } ON DUPLICATE KEY UPDATE position = position;

-- =============================================================
-- Transaction ID deduplication (per device)
-- =============================================================
DEFINE TABLE IF NOT EXISTS txn_id SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device_id  ON TABLE txn_id TYPE string;
DEFINE FIELD IF NOT EXISTS room_id    ON TABLE txn_id TYPE string;
DEFINE FIELD IF NOT EXISTS txn_id     ON TABLE txn_id TYPE string;
DEFINE FIELD IF NOT EXISTS event_id   ON TABLE txn_id TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE txn_id TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_txn_id_unique ON TABLE txn_id FIELDS device_id, room_id, txn_id UNIQUE;

-- =============================================================
-- GRAPH: Event DAG edges (prev_events, auth_events)
-- =============================================================
-- event:A --event_edge--> event:B means A references B
-- Auth chain: SELECT ->event_edge[WHERE edge_type='auth']->event.* FROM event:$id
-- Backfill:   SELECT <-event_edge[WHERE edge_type='prev']<-event.* FROM event:$tip
DEFINE TABLE IF NOT EXISTS event_edge TYPE RELATION IN event OUT event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS edge_type ON TABLE event_edge TYPE string;

-- =============================================================
-- GRAPH: Room upgrade chain (old_room --upgrades_to--> new_room)
-- =============================================================
-- Traverse forward:  SELECT ->upgrades_to->room.* FROM room:$old
-- Traverse backward: SELECT <-upgrades_to<-room.* FROM room:$new
-- Full chain:        SELECT ->upgrades_to->room->upgrades_to->room.* FROM room:$oldest
DEFINE TABLE IF NOT EXISTS upgrades_to TYPE RELATION IN room OUT room SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS version             ON TABLE upgrades_to TYPE string;    -- target room version
DEFINE FIELD IF NOT EXISTS creator             ON TABLE upgrades_to TYPE string;    -- who initiated the upgrade
DEFINE FIELD IF NOT EXISTS tombstone_event_id  ON TABLE upgrades_to TYPE string;    -- tombstone event in old room
DEFINE FIELD IF NOT EXISTS created_at          ON TABLE upgrades_to TYPE datetime DEFAULT time::now();

-- =============================================================
-- GRAPH: Event relations (threads, reactions, edits, references)
-- =============================================================
-- event:reaction --relates_to--> event:parent
-- Thread tree: SELECT <-relates_to[WHERE rel_type='m.thread']<-event.* FROM event:$root
-- Reactions:   SELECT <-relates_to[WHERE rel_type='m.annotation'].content_key FROM event:$target
DEFINE TABLE IF NOT EXISTS relates_to TYPE RELATION IN event OUT event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS rel_type    ON TABLE relates_to TYPE string;
DEFINE FIELD IF NOT EXISTS room_id     ON TABLE relates_to TYPE string;
DEFINE FIELD IF NOT EXISTS sender      ON TABLE relates_to TYPE string;
DEFINE FIELD IF NOT EXISTS event_type  ON TABLE relates_to TYPE string;
DEFINE FIELD IF NOT EXISTS content_key     ON TABLE relates_to TYPE option<string>;
DEFINE FIELD IF NOT EXISTS child_event_id  ON TABLE relates_to TYPE option<string>;
DEFINE FIELD IF NOT EXISTS parent_event_id ON TABLE relates_to TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at      ON TABLE relates_to TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_relates_to_room_type ON TABLE relates_to FIELDS room_id, rel_type;

-- =============================================================
-- Server signing keys
-- =============================================================
DEFINE TABLE IF NOT EXISTS server_key SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS key_id      ON TABLE server_key TYPE string;
DEFINE FIELD IF NOT EXISTS algorithm   ON TABLE server_key TYPE string DEFAULT "ed25519";
DEFINE FIELD IF NOT EXISTS public_key  ON TABLE server_key TYPE string;
DEFINE FIELD IF NOT EXISTS private_key ON TABLE server_key TYPE string;
DEFINE FIELD IF NOT EXISTS valid_until ON TABLE server_key TYPE datetime;
DEFINE FIELD IF NOT EXISTS created_at  ON TABLE server_key TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_server_key_key_id ON TABLE server_key FIELDS key_id UNIQUE;

-- =============================================================
-- Read receipts
-- =============================================================
DEFINE TABLE IF NOT EXISTS receipt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user_id       ON TABLE receipt TYPE string;
DEFINE FIELD IF NOT EXISTS room_id       ON TABLE receipt TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_type  ON TABLE receipt TYPE string;
DEFINE FIELD IF NOT EXISTS event_id      ON TABLE receipt TYPE string;
DEFINE FIELD IF NOT EXISTS ts            ON TABLE receipt TYPE int;
DEFINE INDEX IF NOT EXISTS idx_receipt_user_room_type ON TABLE receipt FIELDS user_id, room_id, receipt_type UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_receipt_room           ON TABLE receipt FIELDS room_id;

-- =============================================================
-- E2EE: Device keys
-- =============================================================
DEFINE TABLE IF NOT EXISTS device_key SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user_id     ON TABLE device_key TYPE string;
DEFINE FIELD IF NOT EXISTS device_id   ON TABLE device_key TYPE string;
DEFINE FIELD IF NOT EXISTS algorithms  ON TABLE device_key TYPE array;
DEFINE FIELD IF NOT EXISTS algorithms.* ON TABLE device_key TYPE string;
DEFINE FIELD IF NOT EXISTS keys        ON TABLE device_key TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS signatures  ON TABLE device_key TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS updated_at  ON TABLE device_key TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_device_key_user_device ON TABLE device_key FIELDS user_id, device_id UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_device_key_user        ON TABLE device_key FIELDS user_id;

-- =============================================================
-- E2EE: One-time keys
-- =============================================================
DEFINE TABLE IF NOT EXISTS one_time_key SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user_id   ON TABLE one_time_key TYPE string;
DEFINE FIELD IF NOT EXISTS device_id ON TABLE one_time_key TYPE string;
DEFINE FIELD IF NOT EXISTS key_id    ON TABLE one_time_key TYPE string;
DEFINE FIELD IF NOT EXISTS key_data  ON TABLE one_time_key TYPE object FLEXIBLE;

DEFINE INDEX IF NOT EXISTS idx_otk_user_device ON TABLE one_time_key FIELDS user_id, device_id;
DEFINE INDEX IF NOT EXISTS idx_otk_unique      ON TABLE one_time_key FIELDS user_id, device_id, key_id UNIQUE;

-- =============================================================
-- E2EE: Cross-signing keys
-- =============================================================
DEFINE TABLE IF NOT EXISTS cross_signing_key SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user_id  ON TABLE cross_signing_key TYPE string;
DEFINE FIELD IF NOT EXISTS key_type ON TABLE cross_signing_key TYPE string;
DEFINE FIELD IF NOT EXISTS key_data ON TABLE cross_signing_key TYPE object FLEXIBLE;

DEFINE INDEX IF NOT EXISTS idx_cross_signing_user_type ON TABLE cross_signing_key FIELDS user_id, key_type UNIQUE;

-- =============================================================
-- E2EE: Key signatures
-- =============================================================
DEFINE TABLE IF NOT EXISTS key_signature SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS signed_user_id  ON TABLE key_signature TYPE string;
DEFINE FIELD IF NOT EXISTS signed_key_id   ON TABLE key_signature TYPE string;
DEFINE FIELD IF NOT EXISTS signing_user_id ON TABLE key_signature TYPE string;
DEFINE FIELD IF NOT EXISTS signing_key_id  ON TABLE key_signature TYPE string;
DEFINE FIELD IF NOT EXISTS signature       ON TABLE key_signature TYPE string;

DEFINE INDEX IF NOT EXISTS idx_key_sig_signed ON TABLE key_signature FIELDS signed_user_id, signed_key_id;

-- =============================================================
-- To-device messages
-- =============================================================
DEFINE TABLE IF NOT EXISTS to_device_message SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS target_user_id   ON TABLE to_device_message TYPE string;
DEFINE FIELD IF NOT EXISTS target_device_id ON TABLE to_device_message TYPE string;
DEFINE FIELD IF NOT EXISTS sender           ON TABLE to_device_message TYPE string;
DEFINE FIELD IF NOT EXISTS event_type       ON TABLE to_device_message TYPE string;
DEFINE FIELD IF NOT EXISTS content          ON TABLE to_device_message TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS stream_position  ON TABLE to_device_message TYPE int;

DEFINE INDEX IF NOT EXISTS idx_todevice_target ON TABLE to_device_message FIELDS target_user_id, target_device_id, stream_position;

-- =============================================================
-- Account Data (global and per-room)
-- =============================================================
DEFINE TABLE IF NOT EXISTS account_data SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user_id   ON TABLE account_data TYPE string;
DEFINE FIELD IF NOT EXISTS room_id   ON TABLE account_data TYPE option<string>;
DEFINE FIELD IF NOT EXISTS data_type ON TABLE account_data TYPE string;
DEFINE FIELD IF NOT EXISTS content   ON TABLE account_data TYPE object FLEXIBLE;

DEFINE INDEX IF NOT EXISTS idx_account_data_unique ON TABLE account_data FIELDS user_id, room_id, data_type UNIQUE;

-- =============================================================
-- Federation: transaction deduplication
-- =============================================================
DEFINE TABLE IF NOT EXISTS federation_txn SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS origin      ON TABLE federation_txn TYPE string;
DEFINE FIELD IF NOT EXISTS txn_id      ON TABLE federation_txn TYPE string;
DEFINE FIELD IF NOT EXISTS received_at ON TABLE federation_txn TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_fed_txn_unique ON TABLE federation_txn FIELDS origin, txn_id UNIQUE;

-- =============================================================
-- Federation: remote server key cache
-- =============================================================
DEFINE TABLE IF NOT EXISTS remote_server_key SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS server_name ON TABLE remote_server_key TYPE string;
DEFINE FIELD IF NOT EXISTS key_id      ON TABLE remote_server_key TYPE string;
DEFINE FIELD IF NOT EXISTS public_key  ON TABLE remote_server_key TYPE string;
DEFINE FIELD IF NOT EXISTS valid_until ON TABLE remote_server_key TYPE datetime;
DEFINE FIELD IF NOT EXISTS fetched_at  ON TABLE remote_server_key TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_remote_key_server_id ON TABLE remote_server_key FIELDS server_name, key_id UNIQUE;

-- =============================================================
-- Media metadata
-- =============================================================
DEFINE TABLE IF NOT EXISTS media SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS media_id       ON TABLE media TYPE string;
DEFINE FIELD IF NOT EXISTS server_name    ON TABLE media TYPE string;
DEFINE FIELD IF NOT EXISTS user_id        ON TABLE media TYPE string;
DEFINE FIELD IF NOT EXISTS content_type   ON TABLE media TYPE string;
DEFINE FIELD IF NOT EXISTS content_length ON TABLE media TYPE int;
DEFINE FIELD IF NOT EXISTS filename       ON TABLE media TYPE option<string>;
DEFINE FIELD IF NOT EXISTS s3_key         ON TABLE media TYPE string;
DEFINE FIELD IF NOT EXISTS quarantined    ON TABLE media TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS created_at     ON TABLE media TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_media_server_id ON TABLE media FIELDS server_name, media_id UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_media_user      ON TABLE media FIELDS user_id;
DEFINE INDEX IF NOT EXISTS idx_media_created   ON TABLE media FIELDS created_at;

-- =============================================================
-- Event reports (abuse reporting)
-- =============================================================
DEFINE TABLE IF NOT EXISTS event_report SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS event_id   ON TABLE event_report TYPE string;
DEFINE FIELD IF NOT EXISTS room_id    ON TABLE event_report TYPE string;
DEFINE FIELD IF NOT EXISTS reporter   ON TABLE event_report TYPE string;
DEFINE FIELD IF NOT EXISTS reason     ON TABLE event_report TYPE option<string>;
DEFINE FIELD IF NOT EXISTS score      ON TABLE event_report TYPE int DEFAULT -100;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE event_report TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_report_room ON TABLE event_report FIELDS room_id;

-- =============================================================
-- Pushers
-- =============================================================
DEFINE TABLE IF NOT EXISTS pusher SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user_id     ON TABLE pusher TYPE string;
DEFINE FIELD IF NOT EXISTS access_token ON TABLE pusher TYPE string;
DEFINE FIELD IF NOT EXISTS pushkey     ON TABLE pusher TYPE string;
DEFINE FIELD IF NOT EXISTS kind        ON TABLE pusher TYPE string;
DEFINE FIELD IF NOT EXISTS app_id      ON TABLE pusher TYPE string;
DEFINE FIELD IF NOT EXISTS app_display_name ON TABLE pusher TYPE option<string>;
DEFINE FIELD IF NOT EXISTS device_display_name ON TABLE pusher TYPE option<string>;
DEFINE FIELD IF NOT EXISTS lang        ON TABLE pusher TYPE option<string>;
DEFINE FIELD IF NOT EXISTS data        ON TABLE pusher TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS created_at  ON TABLE pusher TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_pusher_user ON TABLE pusher FIELDS user_id;
DEFINE INDEX IF NOT EXISTS idx_pusher_user_pushkey ON TABLE pusher FIELDS user_id, pushkey UNIQUE;
