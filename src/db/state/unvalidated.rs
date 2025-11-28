

/// An unvalidated `key`, which may or may not correspond to a row in the 'Records' table.
///
/// Keys are represented as hexadecimal strings prefixed by the `#` character, and can be parsed in a
/// case-insensitive fashion.
///
/// In principle, keys could be negative, but in practice, SQLite will never insert negative values
/// for the key automatically (unless negative keys are already present), and we never manually input values for the `key` column.
#[allow(unused)]
pub struct Unvalidated(i64);

impl std::str::FromStr for Unvalidated {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        i64::from_str_radix(s, 16).map(Self)
    }
}
