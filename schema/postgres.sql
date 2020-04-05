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