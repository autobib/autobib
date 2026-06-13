use std::str::from_utf8;

use typst_syntax::{
    SyntaxNode,
    ast::{self, Arg, Expr},
    parse,
};

use crate::{RecordId, logger::error};

/// Get all citation keys in the buffer.
///
/// This intentionally supports only explicit calls to Typst's `cite` function, like `#cite(<key>)` and
/// `#cite(label("key"))`. For now, we ignore the shorthand `@key` syntax since this is ambiguous with
/// internal Typst labels.
pub fn get_citekeys<T: Extend<RecordId>>(buffer: &[u8], container: &mut T) {
    match from_utf8(buffer) {
        Ok(contents) => {
            let root = parse(contents);
            collect_citekeys(&root, container);
        }
        Err(err) => {
            error!("Typst source file is not valid UTF-8: {}", err);
        }
    }
}

fn collect_citekeys<T: Extend<RecordId>>(node: &SyntaxNode, container: &mut T) {
    if let Some(call) = node.cast::<ast::FuncCall>()
        && is_named_call(call, "cite")
        && let Some(key) = call.args().items().find_map(from_arg)
    {
        container.extend([key]);
    }

    for child in node.children() {
        collect_citekeys(child, container);
    }
}

fn from_arg(arg: Arg<'_>) -> Option<RecordId> {
    let Arg::Pos(expr) = arg else {
        return None;
    };

    from_expr(expr)
}

fn from_expr(expr: Expr<'_>) -> Option<RecordId> {
    match expr {
        Expr::Label(label) => Some(RecordId::from(label.get())),
        // unfortunately, it seems to be necessary to manually parse label(...) arguments
        // since these are not convertex automatically by `typst-syntax`.
        Expr::FuncCall(call) if is_named_call(call, "label") => {
            // FIXME: multiple args?
            call.args().items().find_map(from_string_arg)
        }
        _ => None,
    }
}

fn from_string_arg(arg: Arg<'_>) -> Option<RecordId> {
    let Arg::Pos(Expr::Str(string)) = arg else {
        return None;
    };

    let key = string.get();
    (!key.is_empty()).then(|| RecordId::from(key.as_str()))
}

fn is_named_call(call: ast::FuncCall<'_>, name: &str) -> bool {
    matches!(call.callee(), Expr::Ident(ident) if ident.as_str() == name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn test_get_citekeys_typ() {
        let input = br#"
This reference is intentionally ignored: @internal-label.
#cite(<zbl:1337.28015>)
#cite(label("doi:10.4007/annals.2014.180.2.7"), form: "prose")
#cite(label("escaped\"key"))
#cite(<mr:3224722>)[p. 7]
#cite(label(""))
#not-cite(<ignored>)
`#cite(<raw>)`
// #cite(<comment>)
"#;

        let mut keys = BTreeSet::new();
        get_citekeys(input, &mut keys);

        let expected = [
            "doi:10.4007/annals.2014.180.2.7",
            "escaped\"key",
            "mr:3224722",
            "zbl:1337.28015",
        ];
        assert_eq!(keys.len(), expected.len());
        for key in expected {
            assert!(keys.contains(&RecordId::from(key)));
        }
    }
}
