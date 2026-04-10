//! Storage abstraction layer for Maelstrom.
//!
//! This crate defines the persistence interface that the rest of the server depends on.
//! No handler, service, or federation module ever talks to a database directly -- every
//! read and write goes through the [`Storage`](traits::Storage) super-trait defined in
//! [`traits`].
//!
//! # Architecture
//!
//! ```text
//!   Handler / Service code
//!          |
//!          v
//!    Storage trait  (traits.rs)
//!     /          \
//!    v            v
//! SurrealStorage  MockStorage
//! (surreal/)      (mock.rs)
//! ```
//!
//! * **[`traits`]** -- Trait definitions (`UserStore`, `RoomStore`, `EventStore`, ...),
//!   shared record types (`UserRecord`, `DeviceRecord`, etc.), and the `StorageError` enum.
//!   This is the contract that every backend must satisfy.
//!
//! * **[`surreal`]** -- The production backend, backed by SurrealDB. It connects over
//!   WebSocket (or in-memory for tests), bootstraps the schema on startup, and implements
//!   every sub-trait with SurrealQL queries.
//!
//! * **[`mock`]** -- A lightweight, in-memory implementation using `HashMap`/`HashSet`
//!   behind `Mutex`. Used exclusively in integration tests so they run without a real
//!   database.
//!
//! # Adding a new storage operation
//!
//! 1. Add the method to the appropriate sub-trait in `traits.rs`.
//! 2. Implement it in `surreal/<module>.rs`.
//! 3. Implement it in `mock.rs`.
//! 4. Write tests against `MockStorage` in the `tests/` directory.

pub mod mock;
pub mod traits;

pub mod surreal;

pub use surreal::SurrealStorage;
