/// A representation of database state, to which operations can be applied, and which
/// committed to save the changes to the database.
pub trait DatabaseState {
    /// Commit the underlying transaction.
    fn commit(self) -> Result<(), rusqlite::Error>
    where
        Self: Sized;

    /// Apply an operation in the current state.
    #[inline]
    fn apply<T, O: FnOnce(&Self) -> Result<T, rusqlite::Error>>(
        &self,
        operation: O,
    ) -> Result<T, rusqlite::Error> {
        operation(self)
    }
}

/// Macro to implement [`DatabaseState`] for the relvant types in this module.
macro_rules! tx_impl {
    ($target:ident) => {
        impl crate::db::state::transaction::DatabaseState for $target<'_> {
            #[inline]
            fn commit(self) -> Result<(), ::rusqlite::Error> {
                ::log::debug!("Committing changes to database.");
                self.tx.commit()
            }
        }
    };
}

pub(super) use tx_impl;
