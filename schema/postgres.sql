-- Much of this was copied from Dendrite
--
----------------
-- Accounts
----------------
DROP TABLE IF EXISTS accounts;
CREATE TABLE IF NOT EXISTS accounts (
  -- The Matrix user ID localpart for this account
  localpart TEXT NOT NULL PRIMARY KEY,
  -- When this account was first created, as a unix timestamp (ms resolution).
  created_ts BIGINT NOT NULL,
  -- The password hash for this account. Can be NULL if this is a passwordless account.
  password_hash TEXT,
  -- Random salt added to password for hashing
  password_salt TEXT,
  -- Identifies which application service this account belongs to, if any.
  appservice_id TEXT,
  -- Is this account a server admin
  is_admin bool DEFAULT FALSE NOT NULL,
  is_guest bool DEFAULT FALSE NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_accounts_is_guest ON accounts(is_guest);

DROP TABLE IF EXISTS account_profiles;
CREATE TABLE IF NOT EXISTS account_profiles (
  -- The Matrix user ID localpart for this account
  localpart TEXT NOT NULL PRIMARY KEY,
  -- The display name for this account
  display_name TEXT,
  -- The URL of the avatar for this account
  avatar_url TEXT
);

----------------
-- Devices
----------------
DROP TABLE IF EXISTS devices;
-- Stores data about devices.
CREATE TABLE IF NOT EXISTS devices (
  -- The access token granted to this device. This has to be the primary key
  -- so we can distinguish which device is making a given request.
  access_token TEXT NOT NULL PRIMARY KEY,
  -- The auto-allocated unique ID of the session identified by the access token.
  -- This can be used as a secure substitution of the access token in situations
  -- where data is associated with access tokens (e.g. transaction storage),
  -- so we don't have to store users' access tokens everywhere.
  session_id UUID NOT NULL,
  -- The device identifier. This only needs to uniquely identify a device for a given user, not globally.
  -- access_tokens will be clobbered based on the device ID for a user.
  device_id TEXT NOT NULL,
  -- The Matrix user ID localpart for this device. This is preferable to storing the full user_id
  -- as it is smaller, makes it clearer that we only manage devices for our own users, and may make
  -- migration to different domain names easier.
  localpart TEXT NOT NULL REFERENCES accounts(localpart) ON DELETE CASCADE ON UPDATE CASCADE,
  -- When this devices was first recognised on the network, as a unix timestamp (ms resolution).
  created_ts BIGINT NOT NULL,
  -- The display name, human friendlier than device_id and updatable
  display_name TEXT
);
CREATE INDEX IF NOT EXISTS idx_devices_localpart ON devices(localpart);
CREATE INDEX IF NOT EXISTS idx_devices_session_id ON devices(session_id);
