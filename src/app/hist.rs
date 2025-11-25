use crate::{
    db::state::{ArbitraryKey, RecordRowMoveResult, RedoError, UndoError},
    logger::{error, suggest},
};

pub fn handle_undo_result<'conn, I, J>(
    res: RecordRowMoveResult<'conn, I, J, UndoError>,
) -> anyhow::Result<()> {
    match res {
        RecordRowMoveResult::Updated(state) => {
            state.commit()?;
        }
        RecordRowMoveResult::Unchanged(state, err) => {
            state.commit()?;
            match err {
                UndoError::ParentExists => {
                    error!("Parent is not deleted");
                }
                UndoError::ParentDeleted => {
                    error!("Cannot undo into a deleted state with `autobib hist undo`",);
                    suggest!("Undo into a deleted state with `autobib hist undo --delete`");
                }
                UndoError::ParentVoidExists => {
                    error!("Cannot void record with `autobib hist undo`",);
                    suggest!("Void records with `autobib hist void`");
                }
                UndoError::ParentVoidMissing => {
                    error!("Cannot void record with `autobib hist undo`",);
                    suggest!("Void records with `autobib hist void`");
                }
            }
        }
    }

    Ok(())
}

pub fn handle_redo_result<'conn, I>(
    res: RecordRowMoveResult<'conn, ArbitraryKey, I, RedoError>,
) -> anyhow::Result<()> {
    match res {
        RecordRowMoveResult::Updated(state) => {
            state.commit()?;
        }
        RecordRowMoveResult::Unchanged(state, redo_err) => {
            state.commit()?;
            match redo_err {
                RedoError::OutOfBounds(0) => {
                    error!("No changes to redo");
                }
                RedoError::OutOfBounds(child_count) => {
                    error!("Index out of range: there only {child_count} divergent changes");
                }
                RedoError::ChildNotUnique(child_count) => {
                    error!("There are {child_count} divergent changes");
                    suggest!(
                        "Review the changes with `autobib log --all` and choose a specific change using the INDEX argument."
                    );
                }
            }
        }
    }
    Ok(())
}
