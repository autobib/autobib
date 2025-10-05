use std::{collections::BTreeSet, convert::Infallible, iter::Peekable, str::FromStr};

use mufmt::{Ast, Manifest, ManifestMut, Span, SyntaxError};
use nucleo_picker::Render;

use crate::{
    db::{CitationKey, state::RowData},
    entry::{EntryData, FieldKey, RawRecordFieldsIter, RecordData},
    error::KeyParseError,
};

/// A basic template token.
#[derive(Debug, Clone)]
pub enum Token {
    /// `{key}`
    Field(FieldKey),
    /// `{"string"}`
    String(String),
    /// `{%entry_type}`
    EntryType,
    /// `{%provider}`
    Provider,
    /// `{%sub_id}`
    SubId,
    /// `{%full_id}`
    FullId,
}

impl Ast<'_> for Token {
    type Error = KeyParseError;

    fn from_expr(expr: &'_ str) -> Result<Self, Self::Error> {
        let mut chars = expr.chars();
        match chars.next() {
            Some('%') => match chars.as_str() {
                "entry_type" => Ok(Self::EntryType),
                "provider" => Ok(Self::Provider),
                "sub_id" => Ok(Self::SubId),
                "full_id" => Ok(Self::FullId),
                _ => Err(KeyParseError::InvalidSpecial(chars.as_str().into())),
            },
            Some('"') => {
                let s = serde_json::from_str(expr)?;
                Ok(Self::String(s))
            }
            _ => {
                let key = FieldKey::try_new(expr)?.to_owned();
                Ok(Self::Field(key))
            }
        }
    }
}

/// The key type, representing a `{ ... }` expression in the template.
///
/// This is either a bare token, or a conditional token which only renders if the key is present in
/// the field keys.
#[derive(Debug, Clone)]
pub enum Key {
    /// `{=key raw}`
    Conditional(FieldKey, Token),
    /// `{raw}`
    Bare(Token),
}

impl Ast<'_> for Key {
    type Error = KeyParseError;

    fn from_expr(expr: &str) -> Result<Self, Self::Error> {
        let mut chars = expr.chars();
        match chars.next() {
            Some('=') => match chars.as_str().split_once(char::is_whitespace) {
                Some((key, s)) => {
                    let conditional = FieldKey::try_new(key)?.to_owned();
                    // 's' is already end-trimmed
                    Ok(Self::Conditional(
                        conditional,
                        Token::from_expr(s.trim_start())?,
                    ))
                }
                None => Err(KeyParseError::IncompleteConditional),
            },
            _ => Ok(Self::Bare(Token::from_expr(expr)?)),
        }
    }
}

/// The strategy to use in the [`Manifest`] implementation.
///
/// - `Sorted`: If the keys in the template are sorted, we can avoid allocating by iterating over the fields
///   simultaneously with the template keys.
/// - `Small`: The number of keys in the template is small (currently, <= 4 unique keys). We render
///   using a brute-force approach.
/// - `Large`: The number of keys in the template is large, and they are not sorted. We then
///   allocate a temporary `RecordData<&'r str>` to hold the key-value pairs, and then search for
///   the values from this temporary struct.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Strategy {
    Sorted,
    Small,
    Large,
}

/// An iterator over the field keys in a template, in order of appearance.
struct TemplateFieldKeys<'a, T> {
    spans: std::slice::Iter<'a, Span<T, Key>>,
    buffered: Option<&'a FieldKey>,
}

impl<'a, T> TemplateFieldKeys<'a, T> {
    pub fn new(template: &'a mufmt::Template<T, Key>) -> Self {
        Self {
            spans: template.spans().iter(),
            buffered: None,
        }
    }
}

impl<'a, T> Iterator for TemplateFieldKeys<'a, T> {
    type Item = &'a FieldKey;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(b) = self.buffered.take() {
            return Some(b);
        }

        loop {
            match self.spans.next()? {
                Span::Expr(Key::Bare(Token::Field(f))) => return Some(f),
                Span::Expr(Key::Conditional(f, raw)) => {
                    if let Token::Field(field_key) = raw {
                        self.buffered = Some(field_key);
                    }
                    return Some(f);
                }
                _ => {}
            }
        }
    }
}

/// A wrapper around a [`mufmt::Template`].
#[derive(Debug, Clone)]
pub struct Template {
    template: mufmt::Template<String, Key>,
    strategy: Strategy,
}

impl Template {
    pub fn has_keys_contained_in(&self, row: &RowData) -> bool {
        match self.strategy {
            Strategy::Sorted => {
                let mut fields = BibtexFields::new(row);
                for span in self.template.spans() {
                    if let Span::Expr(Key::Bare(Token::Field(k))) = span
                        && fields.get_field_ordered(k.as_ref()).is_none()
                    {
                        return false;
                    }
                }
            }
            Strategy::Small => {
                for span in self.template.spans() {
                    if let Span::Expr(Key::Bare(Token::Field(k))) = span
                        && !row.data.contains_field(k.as_ref())
                    {
                        return false;
                    }
                }
            }
            Strategy::Large => {
                let data = RecordData::borrow_entry_data(&row.data);
                for span in self.template.spans() {
                    if let Span::Expr(Key::Bare(Token::Field(k))) = span
                        && !data.contains_field(k.as_ref())
                    {
                        return false;
                    }
                }
            }
        }
        true
    }
}

impl Default for Template {
    fn default() -> Self {
        let template =
            mufmt::Template::compile(r#"{author} ~ {title}{=subtitle ". "}{subtitle}"#).unwrap();

        Self {
            template,
            strategy: Strategy::Small,
        }
    }
}

impl FromStr for Template {
    type Err = SyntaxError<KeyParseError>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let template = mufmt::Template::<String, Key>::compile(s)?;

        let strategy = if TemplateFieldKeys::new(&template).is_sorted() {
            Strategy::Sorted
        } else if TemplateFieldKeys::new(&template)
            .collect::<BTreeSet<&FieldKey<String>>>()
            .len()
            <= 4
        {
            Strategy::Small
        } else {
            Strategy::Large
        };

        Ok(Self { template, strategy })
    }
}

pub struct BibtexFields<'a> {
    inner: Peekable<RawRecordFieldsIter<'a>>,
}

impl<'a> BibtexFields<'a> {
    pub fn new(row: &'a RowData) -> Self {
        Self {
            inner: row.data.raw_fields().peekable(),
        }
    }

    pub fn get_field_ordered(&mut self, key: &str) -> Option<&'a str> {
        // advance the inner iterator until we either find or miss the key
        while self.inner.next_if(|nxt| nxt.0 < key).is_some() {}

        // check the next key: if it
        match self.inner.peek() {
            Some((k, v)) if *k == key => Some(v),
            _ => None,
        }
    }
}

/// A return type which helps the compiler reason about lifetimes.
enum DisplayedRow<'row, 'ast, 'state> {
    Row(&'row str),
    Ast(&'ast str),
    State(&'state str),
    Skip,
}

impl<'r, 'ast, 'state> std::fmt::Display for DisplayedRow<'r, 'ast, 'state> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Row(s) => f.write_str(s),
            Self::Ast(s) => f.write_str(s),
            Self::State(s) => f.write_str(s),
            Self::Skip => Ok(()),
        }
    }
}

impl<'row, 'ast, 'state> DisplayedRow<'row, 'ast, 'state> {
    fn from_data<F>(row_data: &'row RowData, ast: &'ast Key, mut f: F) -> Self
    where
        F: FnMut(&str) -> Option<&'state str>,
    {
        let token = match ast {
            Key::Conditional(field_key, token) => {
                if f(field_key.as_ref()).is_some() {
                    token
                } else {
                    return Self::Skip;
                }
            }
            Key::Bare(token) => token,
        };

        match token {
            Token::Field(key) => match f(key.as_ref()) {
                Some(val) => DisplayedRow::State(val),
                None => DisplayedRow::Skip,
            },
            Token::String(s) => DisplayedRow::Ast(s),
            Token::EntryType => DisplayedRow::Row(row_data.data.entry_type()),
            Token::Provider => DisplayedRow::Row(row_data.canonical.provider()),
            Token::SubId => DisplayedRow::Row(row_data.canonical.sub_id()),
            Token::FullId => DisplayedRow::Row(row_data.canonical.name()),
        }
    }
}

pub struct ManifestSorted<'r>(&'r RowData);

impl<'r> ManifestMut<Key> for ManifestSorted<'r> {
    type Error = Infallible;

    type State<'a> = BibtexFields<'a>;

    fn init_state(&self) -> Self::State<'_> {
        BibtexFields::new(self.0)
    }

    fn manifest_mut(
        &self,
        ast: &Key,
        state: &mut Self::State<'_>,
    ) -> Result<impl std::fmt::Display, Self::Error> {
        Ok(DisplayedRow::from_data(self.0, ast, |k| {
            state.get_field_ordered(k)
        }))
    }
}

pub struct ManifestSmall<'r>(&'r RowData);

impl<'r> Manifest<Key> for ManifestSmall<'r> {
    type Error = Infallible;

    fn manifest(&self, ast: &Key) -> Result<impl std::fmt::Display, Self::Error> {
        Ok(DisplayedRow::from_data(self.0, ast, |k| {
            self.0.data.get_field(k)
        }))
    }
}

pub struct ManifestLarge<'r>(&'r RowData);

impl<'r> ManifestMut<Key> for ManifestLarge<'r> {
    type Error = Infallible;

    type State<'s> = RecordData<&'s str>;

    fn init_state(&self) -> Self::State<'_> {
        RecordData::borrow_entry_data(&self.0.data)
    }

    fn manifest_mut(
        &self,
        ast: &Key,
        state: &mut Self::State<'_>,
    ) -> Result<impl std::fmt::Display, Self::Error> {
        Ok(DisplayedRow::from_data(self.0, ast, |k| state.get_field(k)))
    }
}

impl Render<RowData> for Template {
    type Str<'a> = String;

    fn render<'a>(&self, item: &'a RowData) -> Self::Str<'a> {
        match self.strategy {
            Strategy::Sorted => {
                let Ok(s) = self.template.render(&ManifestSorted(item));
                s
            }
            Strategy::Small => {
                let Ok(s) = self.template.render(&ManifestSmall(item));
                s
            }
            Strategy::Large => {
                let Ok(s) = self.template.render(&ManifestLarge(item));
                s
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{entry::RawRecordData, record::RemoteId};

    use chrono::Local;

    use super::*;

    #[test]
    fn test_field_keys() {
        fn check<const N: usize>(s: &str, keys: [&'static str; N]) {
            let template = Template::from_str(s).unwrap();
            assert_eq!(TemplateFieldKeys::new(&template.template).count(), N);

            let field_keys = TemplateFieldKeys::new(&template.template);
            for (k, v) in field_keys.zip(keys) {
                assert_eq!(k, v);
            }
        }

        check("{a} {b}", ["a", "b"]);
        check(r#"{=a b} {c} {=d "e"}"#, ["a", "b", "c", "d"]);
        check(r#""#, []);
        check(r#"Nothing"#, []);
    }

    #[test]
    fn test_render_row_data() {
        fn check<const N: usize>(
            s: &str,
            keys: [(&'static str, &'static str); N],
            provider: &str,
            sub_id: &str,
            strategy: Strategy,
            rendered: &str,
        ) {
            println!("Testing template: {s}");

            let template = Template::from_str(s).unwrap();
            let mut data = RecordData::<String>::default();
            for (k, v) in keys {
                println!("Inserting key {k} = {v}");
                data.check_and_insert(k.into(), v.into()).unwrap();
            }

            let row_data = RowData {
                data: RawRecordData::from_entry_data(&data),
                canonical: RemoteId::from_parts(provider, sub_id).unwrap(),
                modified: Local::now(),
            };

            println!("{:?}", row_data.data.get_field("b"));
            println!("{:?}", RecordData::from_entry_data(&row_data.data));

            assert_eq!(template.strategy, strategy);
            assert_eq!(template.render(&row_data), rendered);
        }

        check(
            "{a} {b}",
            [("a", "A"), ("b", "B")],
            "local",
            "12345",
            Strategy::Sorted,
            "A B",
        );

        check(
            "{b} {a}",
            [("a", "A"), ("b", "B")],
            "local",
            "12345",
            Strategy::Small,
            "B A",
        );

        check(
            "{b} {%sub_id}",
            [("a", "A")],
            "local",
            "12345",
            Strategy::Sorted,
            " 12345",
        );

        check(
            "{=b %sub_id}{=a %provider}",
            [("a", "A")],
            "local",
            "12345",
            Strategy::Small,
            "local",
        );

        check(
            "{=b %sub_id}{=a %provider}{c}{d}{e}{a}",
            [("a", "A")],
            "local",
            "12345",
            Strategy::Large,
            "localA",
        );

        check(
            "{a}{=a a}{a}{a}{b}",
            [("a", "A")],
            "local",
            "12345",
            Strategy::Sorted,
            "AAAA",
        );
    }
}
