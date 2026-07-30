use crate::db::state::RevisionId;

use super::{SelectOneUnchecked, SelectStatement, col};

pub struct GetArbitraryRecord;
impl SelectStatement for GetArbitraryRecord {
    type Args<'a> = RevisionId;
    const STATEMENT: &str = "
    SELECT data, canonical, modified, variant \
      FROM Records \
      WHERE rev = ?1";
    fn args_to_params(args: Self::Args<'_>) -> impl rusqlite::Params {
        [args]
    }
}
impl col::Modified for GetArbitraryRecord {}
impl col::Canonical for GetArbitraryRecord {}
impl col::Variant for GetArbitraryRecord {}
impl col::DataArbitrary for GetArbitraryRecord {}
impl SelectOneUnchecked for GetArbitraryRecord {}

pub struct GetHist;
impl SelectStatement for GetHist {
    type Args<'a> = RevisionId;
    const STATEMENT: &str = "
    SELECT data, canonical, modified, variant, parent_rev \
      FROM Records \
      WHERE rev = ?1";
    fn args_to_params(args: Self::Args<'_>) -> impl rusqlite::Params {
        [args]
    }
}
impl col::Modified for GetHist {}
impl col::Canonical for GetHist {}
impl col::Parent for GetHist {}
impl col::DataArbitrary for GetHist {}
impl SelectOneUnchecked for GetHist {}

pub struct SelectChildren;
impl SelectStatement for SelectChildren {
    type Args<'a> = RevisionId;
    const STATEMENT: &str = "
    SELECT rev, canonical, modified, data, variant, parent_rev
      FROM Records \
      WHERE parent_rev = ?1";
    fn args_to_params(args: Self::Args<'_>) -> impl rusqlite::Params {
        [args]
    }
}
impl col::Rev for SelectChildren {}
impl col::Canonical for SelectChildren {}
impl col::Modified for SelectChildren {}
impl col::Parent for SelectChildren {}
impl col::DataArbitrary for SelectChildren {}

pub struct SelectActiveRecords;
impl SelectStatement for SelectActiveRecords {
    type Args<'a> = ();
    const STATEMENT: &str = "
    SELECT canonical, modified, data
    FROM Records
    WHERE
      rev IN (SELECT record_rev FROM Keys) \
      AND variant = 0";

    fn args_to_params((): Self::Args<'_>) -> impl rusqlite::Params {
        []
    }
}
impl col::Canonical for SelectActiveRecords {}
impl col::Modified for SelectActiveRecords {}
impl col::DataEntry for SelectActiveRecords {}
impl col::DataNotVoid for SelectActiveRecords {}
impl col::DataArbitrary for SelectActiveRecords {}

pub struct SelectMatchingCanonicalActiveRecords;
impl SelectStatement for SelectMatchingCanonicalActiveRecords {
    type Args<'a> = &'a str;
    const STATEMENT: &str = "
    SELECT canonical, modified, data
    FROM Records
    WHERE
      rev IN (SELECT record_rev FROM Keys) \
      AND variant = 0 \
      AND canonical GLOB ?1";
    fn args_to_params(args: Self::Args<'_>) -> impl rusqlite::Params {
        [args]
    }
}
impl col::Canonical for SelectMatchingCanonicalActiveRecords {}
impl col::Modified for SelectMatchingCanonicalActiveRecords {}
impl col::DataEntry for SelectMatchingCanonicalActiveRecords {}
impl col::DataNotVoid for SelectMatchingCanonicalActiveRecords {}
impl col::DataArbitrary for SelectMatchingCanonicalActiveRecords {}

pub struct SelectMatchingActiveRecords;
impl SelectStatement for SelectMatchingActiveRecords {
    type Args<'a> = &'a str;
    const STATEMENT: &str = "
    SELECT name, canonical, modified, data
    FROM Records INNER JOIN Keys ON Keys.record_rev = Records.rev
    WHERE
      Records.variant = 0 \
      AND Keys.name GLOB ?1";
    fn args_to_params(args: Self::Args<'_>) -> impl rusqlite::Params {
        [args]
    }
}
impl col::Canonical for SelectMatchingActiveRecords {}
impl col::Modified for SelectMatchingActiveRecords {}
impl col::Name for SelectMatchingActiveRecords {}
impl col::DataEntry for SelectMatchingActiveRecords {}
impl col::DataNotVoid for SelectMatchingActiveRecords {}
impl col::DataArbitrary for SelectMatchingActiveRecords {}

pub struct SelectMatchingCanonical;
impl SelectStatement for SelectMatchingCanonical {
    type Args<'a> = (bool, &'a str);
    const STATEMENT: &str = "
    SELECT canonical
    FROM Records \
    WHERE
      rev IN (SELECT record_rev FROM Keys) \
      AND variant = ?1 \
      AND canonical GLOB ?2";
    fn args_to_params((deleted, glob): Self::Args<'_>) -> impl rusqlite::Params {
        let variant = if deleted { 1 } else { 0 };
        (variant, glob)
    }
}
impl col::Canonical for SelectMatchingCanonical {}

pub struct SelectMatchingKeys;
impl SelectStatement for SelectMatchingKeys {
    type Args<'a> = (bool, &'a str);
    const STATEMENT: &str = "
    SELECT name
    FROM Keys INNER JOIN Records ON Keys.record_rev = Records.rev \
    WHERE
      Records.variant = ?1 \
      AND variant = ?1 \
      AND Keys.name GLOB ?2";
    fn args_to_params((deleted, glob): Self::Args<'_>) -> impl rusqlite::Params {
        let variant = if deleted { 1 } else { 0 };
        (variant, glob)
    }
}
impl col::Name for SelectMatchingKeys {}

pub struct LogBounded;
impl SelectStatement for LogBounded {
    type Args<'a> = Option<u32>;
    // TODO: WHERE variant != 2
    const STATEMENT: &str = "
    SELECT rev, canonical, modified, data, variant \
      FROM Records \
      ORDER BY modified \
      DESC LIMIT ?1";
    fn args_to_params(limit: Self::Args<'_>) -> impl rusqlite::Params {
        (limit.map(i64::from).unwrap_or(-1),)
    }
}
impl col::Modified for LogBounded {}
impl col::Canonical for LogBounded {}
impl col::Parent for LogBounded {}
impl col::Rev for LogBounded {}
impl col::DataArbitrary for LogBounded {}
