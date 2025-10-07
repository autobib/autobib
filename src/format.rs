mod parse;

use std::{convert::Infallible, fmt, iter::Peekable, str::FromStr};

use mufmt::{Ast, Manifest, ManifestMut, Span, SyntaxError};
use nucleo_picker::Render;

use self::parse::{Kind, Lexer, Token};

use crate::{
    db::{CitationKey, state::RowData},
    entry::{EntryData, FieldKey, RawRecordFieldsIter, RecordData},
    error::{ClapTemplateError, KeyParseError, KeyParseErrorKind},
};

/// A `{%meta}` token.
#[derive(Debug, Clone)]
pub enum Meta {
    /// `{%entry_type}`
    EntryType,
    /// `{%provider}`
    Provider,
    /// `{%sub_id}`
    SubId,
    /// `{%full_id}`
    FullId,
}

impl FromStr for Meta {
    type Err = KeyParseErrorKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "entry_type" => Ok(Self::EntryType),
            "provider" => Ok(Self::Provider),
            "sub_id" => Ok(Self::SubId),
            "full_id" => Ok(Self::FullId),
            _ => Err(KeyParseErrorKind::InvalidMeta(s.into())),
        }
    }
}

/// A helper function to construct a KeyParseError when something specific is expected, but
/// something unexpected was received.
fn unexp(msg: &'static str, t: Token<'_>) -> KeyParseError {
    KeyParseError {
        kind: KeyParseErrorKind::Unexpected(msg, t.kind.describe()),
        span: Some(t.span),
    }
}

/// A basic template component.
#[derive(Debug, Clone)]
pub enum Atom {
    /// `{key}`
    FieldKey(FieldKey),
    /// `{key?}`
    FieldKeyOpt(FieldKey),
    /// `{"string"}`
    String(String),
    /// `{%entry_type}`
    Meta(Meta),
}

trait SpannedError<T> {
    fn spanned(self, span: std::ops::Range<usize>) -> Result<T, KeyParseError>;
}

impl<T, E: Into<KeyParseErrorKind>> SpannedError<T> for Result<T, E> {
    fn spanned(self, span: std::ops::Range<usize>) -> Result<T, KeyParseError> {
        self.map_err(|e| KeyParseError {
            kind: e.into(),
            span: Some(span),
        })
    }
}

impl Atom {
    /// Read a single Atom from the provided lexer without consuming past the end of the atom.
    fn from_lexer(lexer: &mut Lexer<'_>) -> Result<Self, KeyParseError> {
        static MSG: &str = "A field key, string, or meta";
        let token = lexer.expect_token(MSG)?;
        match token.kind {
            Kind::String(s) => Ok(Self::String(s)),
            Kind::Ident(s) => {
                let key = FieldKey::try_new_normalize(s)
                    .spanned(token.span)?
                    .to_owned();
                Ok(if lexer.skip_if_opt() {
                    Self::FieldKeyOpt(key)
                } else {
                    Self::FieldKey(key)
                })
            }
            Kind::Meta => {
                static MSG: &str = "an identifier";
                let token = lexer.expect_token(MSG)?;
                match token.kind {
                    Kind::Ident(s) => Ok(Self::Meta(Meta::from_str(s).spanned(token.span)?)),
                    _ => Err(unexp(MSG, token)),
                }
            }
            _ => Err(unexp(MSG, token)),
        }
    }
}

/// The key type, representing a `{ ... }` expression in the template.
///
/// This is either a bare token, or a conditional token which only renders if the key is present in
/// the field keys.
#[derive(Debug, Clone)]
pub enum Expression {
    /// `{=key atom}`
    Conditional(FieldKey, Atom),
    /// `{atom}`
    Bare(Atom),
}

impl Expression {
    fn from_lexer(lexer: &mut Lexer<'_>) -> Result<Self, KeyParseError> {
        let res = if lexer.skip_if_cond() {
            // {=key} but now the = has been consumed

            static MSG: &str = "a field key";
            let token = lexer.expect_token(MSG)?;
            match token.kind {
                Kind::Ident(s) => {
                    let field_key = FieldKey::try_new_normalize(s).spanned(token.span)?;
                    static MSG: &str = "whitespace and then the conditional value";
                    let token = lexer.expect_token(MSG)?;
                    match token.kind {
                        Kind::Whitespace => {
                            let atom = Atom::from_lexer(lexer)?;
                            Self::Conditional(field_key, atom)
                        }
                        _ => return Err(unexp(MSG, token)),
                    }
                }
                _ => return Err(unexp(MSG, token)),
            }
        } else {
            let atom = Atom::from_lexer(lexer)?;
            Self::Bare(atom)
        };

        lexer.expect_eof()?;
        Ok(res)
    }
}

impl Ast<'_> for Expression {
    type Error = KeyParseError;

    fn from_expr(expr: &str) -> Result<Self, Self::Error> {
        let mut lexer = Lexer::new(expr);
        Self::from_lexer(&mut lexer)
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
    spans: std::slice::Iter<'a, Span<T, Expression>>,
    buffered: Option<&'a FieldKey>,
}

impl<'a, T> TemplateFieldKeys<'a, T> {
    pub fn new(template: &'a mufmt::Template<T, Expression>) -> Self {
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
                Span::Expr(Expression::Bare(Atom::FieldKey(f) | Atom::FieldKeyOpt(f))) => {
                    return Some(f);
                }
                Span::Expr(Expression::Conditional(f, raw)) => {
                    if let Atom::FieldKeyOpt(field_key) | Atom::FieldKey(field_key) = raw {
                        self.buffered = Some(field_key);
                    }
                    return Some(f);
                }
                _ => {}
            }
        }
    }
}

/// A wrapper around a [`mufmt::Template`] which also pre-computes an optimal rendering strategy.
#[derive(Debug, Clone)]
pub struct Template {
    template: mufmt::Template<String, Expression>,
    strategy: Strategy,
}

impl Template {
    pub fn compile(s: &str) -> Result<Self, SyntaxError<KeyParseError>> {
        let template = mufmt::Template::<String, Expression>::compile(s)?;

        let strategy = if TemplateFieldKeys::new(&template).is_sorted() {
            Strategy::Sorted
        } else if TemplateFieldKeys::new(&template).count() <= 4 {
            Strategy::Small
        } else {
            Strategy::Large
        };

        Ok(Self { template, strategy })
    }

    fn contained_impl<T>(
        &self,
        init: impl FnOnce() -> T,
        mut contains: impl FnMut(&str, &mut T) -> bool,
    ) -> bool {
        let mut ctx = init();
        for span in self.template.spans() {
            match span {
                Span::Expr(Expression::Bare(Atom::FieldKey(k))) => {
                    if !contains(k.as_ref(), &mut ctx) {
                        return false;
                    }
                }
                Span::Expr(Expression::Conditional(k1, Atom::FieldKey(k2))) => {
                    if contains(k1.as_ref(), &mut ctx) && !contains(k2.as_ref(), &mut ctx) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }

    pub fn has_keys_contained_in(&self, row: &RowData) -> bool {
        match self.strategy {
            Strategy::Sorted => self.contained_impl(
                || BibtexFields::new(row),
                |k, fields| fields.get_field_ordered(k).is_some(),
            ),
            Strategy::Small => self.contained_impl(|| (), |k, ()| row.data.contains_field(k)),
            Strategy::Large => self.contained_impl(
                || RecordData::borrow_entry_data(&row.data),
                |k, data| data.contains_field(k),
            ),
        }
    }
}

pub const DEFAULT_TEMPLATE: &str = r#"{author} ~ {title}{=subtitle ". "}{=subtitle subtitle}"#;

impl Default for Template {
    fn default() -> Self {
        let template = mufmt::Template::compile(DEFAULT_TEMPLATE).unwrap();

        Self {
            template,
            strategy: Strategy::Small,
        }
    }
}

impl From<SyntaxError<KeyParseError>> for ClapTemplateError {
    fn from(e: SyntaxError<KeyParseError>) -> Self {
        Self(e)
    }
}

impl FromStr for Template {
    type Err = ClapTemplateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::compile(s)?)
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

impl<'r, 'ast, 'state> fmt::Display for DisplayedRow<'r, 'ast, 'state> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Row(s) => f.write_str(s),
            Self::Ast(s) => f.write_str(s),
            Self::State(s) => f.write_str(s),
            Self::Skip => Ok(()),
        }
    }
}

impl<'row, 'ast, 'state> DisplayedRow<'row, 'ast, 'state> {
    fn from_data<F>(row_data: &'row RowData, ast: &'ast Expression, mut f: F) -> Self
    where
        F: FnMut(&str) -> Option<&'state str>,
    {
        let token = match ast {
            Expression::Conditional(field_key, token) => {
                if f(field_key.as_ref()).is_some() {
                    token
                } else {
                    return Self::Skip;
                }
            }
            Expression::Bare(token) => token,
        };

        match token {
            Atom::FieldKey(key) | Atom::FieldKeyOpt(key) => match f(key.as_ref()) {
                Some(val) => DisplayedRow::State(val),
                None => DisplayedRow::Skip,
            },
            Atom::String(s) => DisplayedRow::Ast(s),
            Atom::Meta(meta) => match meta {
                Meta::EntryType => DisplayedRow::Row(row_data.data.entry_type()),
                Meta::Provider => DisplayedRow::Row(row_data.canonical.provider()),
                Meta::SubId => DisplayedRow::Row(row_data.canonical.sub_id()),
                Meta::FullId => DisplayedRow::Row(row_data.canonical.name()),
            },
        }
    }
}

pub struct ManifestSorted<'r>(&'r RowData);

impl<'r> ManifestMut<Expression> for ManifestSorted<'r> {
    type Error = Infallible;

    type State<'a> = BibtexFields<'a>;

    fn init_state(&self) -> Self::State<'_> {
        BibtexFields::new(self.0)
    }

    fn manifest_mut(
        &self,
        ast: &Expression,
        state: &mut Self::State<'_>,
    ) -> Result<impl fmt::Display, Self::Error> {
        Ok(DisplayedRow::from_data(self.0, ast, |k| {
            state.get_field_ordered(k)
        }))
    }
}

pub struct ManifestSmall<'r>(&'r RowData);

impl<'r> Manifest<Expression> for ManifestSmall<'r> {
    type Error = Infallible;

    fn manifest(&self, ast: &Expression) -> Result<impl fmt::Display, Self::Error> {
        Ok(DisplayedRow::from_data(self.0, ast, |k| {
            self.0.data.get_field(k)
        }))
    }
}

pub struct ManifestLarge<'r>(&'r RowData);

impl<'r> ManifestMut<Expression> for ManifestLarge<'r> {
    type Error = Infallible;

    type State<'s> = RecordData<&'s str>;

    fn init_state(&self) -> Self::State<'_> {
        RecordData::borrow_entry_data(&self.0.data)
    }

    fn manifest_mut(
        &self,
        ast: &Expression,
        state: &mut Self::State<'_>,
    ) -> Result<impl fmt::Display, Self::Error> {
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
    fn keys_contained_in() {
        fn check<const N: usize>(s: &str, keys: [(&'static str, &'static str); N], expected: bool) {
            println!("Testing template: {s}");

            let template = Template::compile(s).unwrap();
            let mut data = RecordData::<String>::default();
            for (k, v) in keys {
                data.check_and_insert(k.into(), v.into()).unwrap();
            }

            let row_data = RowData {
                data: RawRecordData::from_entry_data(&data),
                canonical: RemoteId::from_parts("local", "123").unwrap(),
                modified: Local::now(),
            };

            assert_eq!(template.has_keys_contained_in(&row_data), expected);
        }

        check("{a} {b}", [("a", "A"), ("b", "")], true);
        check("{a} {=b c}", [("a", "A")], true);
        check("{a} {=b c}", [("a", "A"), ("b", "B")], false);
        check("{a} {=b b}", [("a", "A"), ("b", "B")], true);
        check("{=b a}", [("a", "A"), ("b", "B")], true);
        check("{=c \". \"}", [("a", "A"), ("b", "B")], true);
    }

    #[test]
    fn test_field_keys() {
        fn check<const N: usize>(s: &str, keys: [&'static str; N]) {
            println!("Testing: {s}");
            let template = Template::compile(s).unwrap();
            assert_eq!(TemplateFieldKeys::new(&template.template).count(), N);

            let field_keys = TemplateFieldKeys::new(&template.template);
            for (k, v) in field_keys.zip(keys) {
                assert_eq!(k, v);
            }
        }

        check("{a} {b}", ["a", "b"]);
        check(r#"{=a b} {c} {=d "e"}"#, ["a", "b", "c", "d"]);
        check(r#"{=c d?} {f?}"#, ["c", "d", "f"]);
        check(r#"{=CH D?}"#, ["ch", "d"]);
        check(r#"{E?}"#, ["e"]);
        check(r#"{(E?)}"#, ["e?"]);
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

            let template = Template::compile(s).unwrap();
            let mut data = RecordData::<String>::default();
            for (k, v) in keys {
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
