mod parse;

use std::{convert::Infallible, fmt, io, str::FromStr};

use autobib_entry::{data::EntryData, ident::FieldKey, v1::ArchivedEntryData};
use chrono::{DateTime, Local};
use mufmt::{Ast, Manifest, Span, SyntaxError};
use nucleo_picker::Render;

use self::parse::{Kind, Lexer, Token};

use crate::{
    db::{AsKey, state::Record},
    error::{ClapTemplateError, KeyParseError, KeyParseErrorKind},
    record::{Identifier, KeyedRecord},
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
    /// `{%key}`
    Key,
    /// `{%modified}`
    Modified,
    /// `{%json}`
    Json,
}

impl FromStr for Meta {
    type Err = KeyParseErrorKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "entry_type" => Ok(Self::EntryType),
            "provider" => Ok(Self::Provider),
            "sub_id" => Ok(Self::SubId),
            "full_id" => Ok(Self::FullId),
            "key" => Ok(Self::Key),
            "modified" => Ok(Self::Modified),
            "json" => Ok(Self::Json),
            _ => Err(KeyParseErrorKind::InvalidMeta(s.into())),
        }
    }
}

/// A helper function to construct a KeyParseError when something specific is expected, but
/// something unexpected was received.
///
/// The `msg` is used as the error message and should contain a description of what was expected.
/// The token `t` is the token which was received instead.
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

/// A helper trait which allows adding a span to a KeyParseErrorKind.
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
    /// Read a single atom from the provided lexer without consuming any characters beyond the end
    /// of the atom.
    fn from_lexer(lexer: &mut Lexer<'_>) -> Result<Self, KeyParseError> {
        static MSG: &str = "A field key, string, or meta";
        let token = lexer.expect_token(MSG)?;
        match token.kind {
            Kind::String(s) => Ok(Self::String(s)),
            Kind::Ident(s) => {
                let key = FieldKey::from_str(s).spanned(token.span)?.to_owned();
                Ok(if lexer.skip_if_opt().is_some() {
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

/// An abstract representation of the contents of a `{ ... }` expression in the template.
///
/// This is either a bare token, or a conditional token which only renders if the key is present
/// or not present in the field keys.
#[derive(Debug, Clone)]
pub enum Expression {
    /// `{atom}`: render `atom`
    Bare(Atom),
    /// `{=key atom}`: render `atom` if `key` is present
    IfDefined(FieldKey, Atom),
    /// `{!key atom}`: render `atom` if `key` is not present
    IfUndefined(FieldKey, Atom),
}

impl Ast<'_> for Expression {
    type Error = KeyParseError;

    fn from_expr(expr: &str) -> Result<Self, Self::Error> {
        let mut lexer = Lexer::new(expr);
        let res = match lexer.skip_if_cond() {
            Some(c) => {
                // {=key} but now the = has been consumed
                static MSG: &str = "a field key";
                let token = lexer.expect_token(MSG)?;
                match token.kind {
                    Kind::Ident(s) => {
                        let field_key = FieldKey::from_str(s).spanned(token.span)?;
                        static MSG: &str = "whitespace and then the conditional value";
                        let token = lexer.expect_token(MSG)?;
                        match token.kind {
                            Kind::Whitespace => {
                                let atom = Atom::from_lexer(&mut lexer)?;
                                if c {
                                    Self::IfDefined(field_key, atom)
                                } else {
                                    Self::IfUndefined(field_key, atom)
                                }
                            }
                            _ => return Err(unexp(MSG, token)),
                        }
                    }
                    _ => return Err(unexp(MSG, token)),
                }
            }
            None => {
                let atom = Atom::from_lexer(&mut lexer)?;
                Self::Bare(atom)
            }
        };

        lexer.expect_eof()?;
        Ok(res)
    }
}

/// A wrapper around a [`mufmt::Template`] which also pre-computes an optimal rendering strategy.
#[derive(Debug, Clone)]
pub struct Template {
    template: mufmt::Template<String, Expression>,
}

impl Template {
    /// Compile this template from a template string.
    pub fn compile(s: &str) -> Result<Self, SyntaxError<KeyParseError>> {
        let template = mufmt::Template::<String, Expression>::compile(s)?;

        Ok(Self { template })
    }

    /// Returns whether this template can be rendered by the provided row data without having
    /// any non-optional undefined keys.
    pub fn has_keys_contained_in<T: TemplateData>(&self, row: &T) -> bool {
        let contains = |k| row.data().contains_field(k);
        for span in self.template.spans() {
            match span {
                Span::Expr(Expression::Bare(Atom::FieldKey(k))) if !contains(k.as_ref()) => {
                    return false;
                }
                Span::Expr(Expression::IfDefined(k1, Atom::FieldKey(k2)))
                    if contains(k1.as_ref()) && !contains(k2.as_ref()) =>
                {
                    return false;
                }
                Span::Expr(Expression::IfUndefined(k1, Atom::FieldKey(k2)))
                    if !contains(k1.as_ref()) && !contains(k2.as_ref()) =>
                {
                    return false;
                }
                _ => {}
            }
        }
        true
    }
}

/// The default template used by `autobib find`.
pub const DEFAULT_FIND_TEMPLATE: &str = r#"{author} ~ {title}{=subtitle ". "}{subtitle?}"#;

impl FromStr for Template {
    type Err = ClapTemplateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::compile(s)?)
    }
}

/// A display wrapper for template data and the expression.
struct DataAst<'row, 'ast, T> {
    row: &'row T,
    ast: &'ast Expression,
}

impl<'row, 'ast, T: TemplateData> DataAst<'row, 'ast, T> {
    fn contains_field(&self, k: &FieldKey) -> bool {
        self.row.data().contains_field(k.as_ref())
    }
}

impl<T: TemplateData> fmt::Display for DataAst<'_, '_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let token = match self.ast {
            Expression::Bare(token) => token,
            Expression::IfDefined(field_key, token) if self.contains_field(field_key) => token,
            Expression::IfUndefined(field_key, token) if !self.contains_field(field_key) => token,
            _ => return Ok(()),
        };

        match token {
            Atom::FieldKey(key) | Atom::FieldKeyOpt(key) => {
                match self.row.data().get_field_str(key.as_ref()) {
                    Some(val) => f.write_str(val),
                    None => Ok(()),
                }
            }
            Atom::String(s) => f.write_str(s),
            Atom::Meta(meta) => match meta {
                Meta::EntryType => f.write_str(self.row.data().entry_type().inner()),
                Meta::Provider => f.write_str(self.row.canonical().provider()),
                Meta::SubId => f.write_str(self.row.canonical().sub_id()),
                Meta::FullId => f.write_str(self.row.canonical().as_key()),
                Meta::Key => f.write_str(self.row.key()),
                Meta::Modified => self.row.modified().fmt(f),
                Meta::Json => self.row.record().write_json_fmt(f),
            },
        }
    }
}

pub struct ManifestSmall<'r, T>(&'r T);

impl<'r, T: TemplateData> Manifest<Expression> for ManifestSmall<'r, T> {
    type Error = Infallible;

    fn manifest(&self, ast: &Expression) -> Result<impl fmt::Display, Self::Error> {
        Ok(DataAst { row: self.0, ast })
    }
}

impl Template {
    /// Render the template into a writer using the provided record data.
    pub fn render_io<W: io::Write, T: TemplateData>(
        &self,
        writer: W,
        item: &T,
    ) -> Result<(), io::Error> {
        Ok(self.template.render_io(&ManifestSmall(item), writer)?)
    }
}

impl<T: TemplateData> Render<T> for Template {
    type Str<'a>
        = String
    where
        T: 'a;

    fn render<'a>(&self, item: &'a T) -> Self::Str<'a> {
        let Ok(s) = self.template.render(&ManifestSmall(item));
        s
    }
}

pub trait TemplateData {
    fn key(&self) -> &str;

    fn data(&self) -> &ArchivedEntryData;

    fn canonical(&self) -> Identifier<&str>;

    fn modified(&self) -> DateTime<Local>;

    fn record(&self) -> Record<&ArchivedEntryData, &str> {
        Record {
            data: self.data(),
            canonical: self.canonical(),
            modified: self.modified(),
        }
    }
}

impl TemplateData for Record {
    fn key(&self) -> &str {
        self.canonical.as_key()
    }

    fn data(&self) -> &ArchivedEntryData {
        &self.data
    }

    fn canonical(&self) -> Identifier<&str> {
        self.canonical.as_deref()
    }

    fn modified(&self) -> DateTime<Local> {
        self.modified
    }
}

impl<'r> TemplateData for Record<&'r ArchivedEntryData, &'r str> {
    fn key(&self) -> &str {
        self.canonical.as_key()
    }

    fn data(&self) -> &ArchivedEntryData {
        self.data
    }

    fn canonical(&self) -> Identifier<&str> {
        self.canonical.as_deref()
    }

    fn modified(&self) -> DateTime<Local> {
        self.modified
    }
}

impl<'r> TemplateData for KeyedRecord<Record<&'r ArchivedEntryData, &'r str>, &'r str> {
    fn key(&self) -> &str {
        self.key
    }

    fn data(&self) -> &ArchivedEntryData {
        self.record.data
    }

    fn canonical(&self) -> Identifier<&str> {
        self.record.canonical.as_deref()
    }

    fn modified(&self) -> DateTime<Local> {
        self.record.modified
    }
}

impl TemplateData for KeyedRecord {
    fn key(&self) -> &str {
        &self.key
    }

    fn data(&self) -> &ArchivedEntryData {
        &self.record.data
    }

    fn canonical(&self) -> Identifier<&str> {
        self.record.canonical.as_deref()
    }

    fn modified(&self) -> DateTime<Local> {
        self.record.modified
    }
}

#[cfg(test)]
mod tests {
    use crate::record::Identifier;
    use autobib_entry::{Archive, data::MutableEntryData};

    use chrono::Local;

    use super::*;

    #[test]
    fn keys_contained_in() {
        fn check<const N: usize>(s: &str, keys: [(&'static str, &'static str); N], expected: bool) {
            println!("Testing template: {s}");

            let template = Template::compile(s).unwrap();
            let mut data = MutableEntryData::default();
            for (k, v) in keys {
                data.try_insert(k, v).unwrap();
            }

            let row_data = Record {
                data: ArchivedEntryData::from_entry_data(&data),
                canonical: Identifier::from_parts("local", "123").unwrap(),
                modified: Local::now(),
            };

            assert_eq!(template.has_keys_contained_in(&row_data), expected);
        }

        check("{a} {b}", [("a", "A"), ("b", "")], true);
        check("{a} {=b c}", [("a", "A")], true);
        check("{a} {=b c}", [("a", "A"), ("b", "B")], false);
        check("{!b c}", [("a", "A")], false);
        check("{!b c}", [("b", "B")], true);
        check("{!a a}", [("a", "A")], true);
        check("{!a a}", [], false);
        check("{!b c?}", [("a", "A")], true);
        check("{=a c?}", [("a", "A")], true);
        check("{!b a}", [("a", "A")], true);
        check("{a} {=b c?}", [("a", "A"), ("b", "B")], true);
        check("{a} {=b b}", [("a", "A"), ("b", "B")], true);
        check("{=b a}", [("a", "A"), ("b", "B")], true);
        check("{=c \". \"}", [("a", "A"), ("b", "B")], true);
    }

    #[test]
    fn test_render_row_data() {
        fn check<const N: usize>(
            s: &str,
            keys: [(&'static str, &'static str); N],
            provider: &str,
            sub_id: &str,
            rendered: &str,
        ) {
            println!("Testing template: {s}");

            let template = Template::compile(s).unwrap();
            let mut data = MutableEntryData::default();
            for (k, v) in keys {
                data.try_insert(k, v).unwrap();
            }

            let row_data = Record::<Box<ArchivedEntryData>> {
                data: ArchivedEntryData::from_entry_data(&data),
                canonical: Identifier::from_parts(provider, sub_id).unwrap(),
                modified: Local::now(),
            };

            println!("{:?}", row_data.data.get_field("b"));
            println!("{:?}", MutableEntryData::from_entry_data(&row_data.data));

            assert_eq!(template.render(&row_data), rendered);
        }

        check("{a} {b}", [("a", "A"), ("b", "B")], "local", "12345", "A B");

        check("{b} {a}", [("a", "A"), ("b", "B")], "local", "12345", "B A");

        check("{b} {%sub_id}", [("a", "A")], "local", "12345", " 12345");

        check(
            "{=b %sub_id}{=a %provider}",
            [("a", "A")],
            "local",
            "12345",
            "local",
        );

        check(
            "{=b %sub_id}{=a %provider}{c}{d}{e}{a}{f}",
            [("a", "A")],
            "local",
            "12345",
            "localA",
        );

        check("{a}{=a a}{a}{a}{b}", [("a", "A")], "local", "12345", "AAAA");
    }

    #[test]
    fn render_json_meta() {
        let template = Template::compile("{%json}").unwrap();
        let mut data = MutableEntryData::default();
        data.try_insert("a", "A").unwrap();
        data.try_insert("b", "B").unwrap();

        let row_data = Record::<Box<ArchivedEntryData>> {
            data: ArchivedEntryData::from_entry_data(&data),
            canonical: Identifier::from_parts("local", "12345").unwrap(),
            modified: Local::now(),
        };

        let rendered = template.render(&row_data);

        assert!(rendered.starts_with(r#"{"data":{"entry_type":"misc","fields":{"a":"A","b":"B"}},"canonical":"local:12345","modified":"#));
    }
}
