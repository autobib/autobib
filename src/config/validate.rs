use std::{fmt, path::Path};

use anyhow::Error;
use regex_syntax::ast::{parse::Parser, Ast, GroupKind, Span};

use super::RawConfig;
use crate::{logger::error, provider::is_valid_provider};

/// Validate the configuration file loaded at the provided path.
///
/// An explicit error is returned if configuration loading fails; otherwise, errors
/// are simply printed to STDERR using the [`logger::error`](crate::logger::error)
/// macro.
pub fn report_config_errors<P: AsRef<Path>>(path: P) -> Result<(), Error> {
    let raw_config = RawConfig::load(path, true)?;

    validate_alias_transform_rules(raw_config.alias_transform.rules);

    Ok(())
}

/// One of the various errors that can occur in an invalid transformation.
#[derive(Debug, PartialEq)]
enum CapturesErrorKind {
    /// An alternation has a variant which is missing a capture group.
    Missing,
    /// A concatenation has too many capture groups.
    TooMany,
    /// A capture group contains another capture group.
    Nested,
}

impl fmt::Display for CapturesErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CapturesErrorKind::Missing => "has variant without capture group",
            CapturesErrorKind::TooMany => "contains more than one capture group",
            CapturesErrorKind::Nested => "contains nested capture group",
        })
    }
}

/// The result of evaluating an [`Ast`] for the presence of capture groups.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// There are no capture groups.
    NoCapture,
    /// Every alternative contains exactly one capture group.
    OneCapture,
    /// Something is invalid; it occurred at the given span.
    Invalid(CapturesErrorKind, usize, usize),
}

impl Outcome {
    /// Construct an `Invalid` from the provided [`Span`].
    fn invalid(kind: CapturesErrorKind, span: Span) -> Self {
        Self::Invalid(kind, span.start.offset, span.end.offset)
    }
}

/// Returns if the [`Ast`] does not contain a capturing group.
///
/// This is equivalent to `eval_ast(ast: &ast) == Outcome::NoCapture`, but with better
/// short-circuiting.
fn has_no_capture_group(ast: &Ast) -> bool {
    match ast {
        Ast::Group(group) => match group.kind {
            GroupKind::NonCapturing(_) => has_no_capture_group(&group.ast),
            _ => false,
        },
        Ast::Alternation(alternation) => alternation.asts.iter().all(has_no_capture_group),
        Ast::Concat(concat) => concat.asts.iter().all(has_no_capture_group),
        _ => true,
    }
}

/// Evaluate the regex [`Ast`] and determine the outcome of evaluation.
///
/// Uses single-pass DFS with short-circuiting.
fn eval_ast(ast: &Ast) -> Outcome {
    match ast {
        Ast::Group(group) => match group.kind {
            // a non-capturing group is transparent
            GroupKind::NonCapturing(_) => eval_ast(&group.ast),
            // we just saw a capturing group, so either the child has no capturing groups, or
            // the ast is invalid
            _ => {
                if has_no_capture_group(&group.ast) {
                    Outcome::OneCapture
                } else {
                    Outcome::invalid(CapturesErrorKind::Nested, group.span)
                }
            }
        },
        Ast::Alternation(alternation) => {
            // every child is NoCapture => NoCapture
            // every child is OneCapture => OneCapture
            // else => Invalid
            let mut children = alternation.asts.iter();
            match children.next() {
                Some(ast) => match eval_ast(ast) {
                    Outcome::NoCapture => {
                        for ast in children {
                            match eval_ast(ast) {
                                Outcome::NoCapture => {}
                                Outcome::OneCapture => {
                                    return Outcome::invalid(
                                        CapturesErrorKind::Missing,
                                        alternation.span,
                                    )
                                }
                                e => return e,
                            }
                        }
                        Outcome::NoCapture
                    }
                    Outcome::OneCapture => {
                        for ast in children {
                            match eval_ast(ast) {
                                Outcome::OneCapture => {}
                                Outcome::NoCapture => {
                                    return Outcome::invalid(
                                        CapturesErrorKind::Missing,
                                        alternation.span,
                                    )
                                }
                                e => return e,
                            }
                        }
                        Outcome::OneCapture
                    }
                    e => e,
                },
                None => Outcome::NoCapture,
            }
        }
        Ast::Concat(concat) => {
            // every child is NoCapture => NoCapture
            // one child is OneCapture, rest NoCapture => OneCapture
            // else => Invalid
            let mut outcome = Outcome::NoCapture;
            for ast in concat.asts.iter() {
                match (&outcome, eval_ast(ast)) {
                    (_, Outcome::NoCapture) => {}
                    (Outcome::NoCapture, Outcome::OneCapture) => {
                        outcome = Outcome::OneCapture;
                    }
                    (Outcome::OneCapture, Outcome::OneCapture) => {
                        return Outcome::invalid(CapturesErrorKind::TooMany, concat.span)
                    }
                    // the pattern guarantees that e is a `Outcome::Invalid`
                    (_, e) => return e,
                }
            }
            outcome
        }
        // none of the other nodes are recursive
        _ => Outcome::NoCapture,
    }
}

/// Validate alias transform rules for correctness; namely regexes compile, providers are valid,
/// and the regex rules satisfy the 'every alternative contains exactly one capture group' rule
fn validate_alias_transform_rules<S: AsRef<str>, T: AsRef<str>>(
    rules: impl IntoIterator<Item = (S, T)>,
) {
    for (re, provider) in rules {
        let provider = provider.as_ref();
        let re = re.as_ref();
        if !is_valid_provider(provider) {
            error!(
                "Config 'alias_transform.rules' rule [\"{re}\", \"{provider}\"]: contains invalid provider"
            );
        }
        match Parser::new().parse(re) {
            Ok(ast) => match eval_ast(&ast) {
                Outcome::NoCapture => {
                    error!("Config 'alias_transform.rules' rule [\"{re}\", \"{provider}\"]: regex does not contain any capture groups");
                }
                Outcome::Invalid(kind, start, end) => {
                    error!(
                        "Config 'alias_transform.rules' rule [\"{re}\", \"{provider}\"]: regex component '{}' {}",
                        &re[start..end],
                        kind,
                    );
                }
                _ => {}
            },
            Err(e) => {
                error!("Config 'alias_transform.rules' rule [\"{re}\", \"{provider}\"]: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_capture_groups() {
        fn assert_caps(re: &str, expected: Outcome) {
            println!("{re}");
            let ast = Parser::new().parse(re).unwrap();
            println!("{ast:?}");
            assert_eq!(eval_ast(&ast), expected);
        }

        assert_caps("a(b)", Outcome::OneCapture);
        assert_caps("(b)", Outcome::OneCapture);
        assert_caps("(a)|(b)", Outcome::OneCapture);
        assert_caps("(a)|(b|c)", Outcome::OneCapture);
        assert_caps("(a)|(?:(b)|c(d))", Outcome::OneCapture);
        assert_caps("a(?:(b)|c(d))", Outcome::OneCapture);
        assert_caps("(?i)a+((?-i)b+)", Outcome::OneCapture);
        assert_caps("((?i)a+(?-i)b+)", Outcome::OneCapture);

        assert_caps("a", Outcome::NoCapture);
        assert_caps("a(?:b|c|d)", Outcome::NoCapture);
        assert_caps("a", Outcome::NoCapture);

        assert_caps(
            "(a)(b(?:c))",
            Outcome::Invalid(CapturesErrorKind::TooMany, 0, 11),
        );
        assert_caps("(a)(b)", Outcome::Invalid(CapturesErrorKind::TooMany, 0, 6));
        assert_caps(
            "(a)(b(c))",
            Outcome::Invalid(CapturesErrorKind::Nested, 3, 9),
        );
        assert_caps(
            "(a)|(?:b|c(d))",
            Outcome::Invalid(CapturesErrorKind::Missing, 7, 13),
        );

        assert_caps(
            "a(?:b|c(d))",
            Outcome::Invalid(CapturesErrorKind::Missing, 4, 10),
        );
    }
}
