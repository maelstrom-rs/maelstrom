pub mod postgres;

pub use postgres::PostgresStore;

/// A Storage Driver.
///
/// This trait encapsulates a complete storage driver to a
/// specific type of storage mechanism, e.g. Postgres, Kafka, etc.
pub trait Store: Clone + Sync + Send + Sized {
    fn get_type(&self) -> String;
}
