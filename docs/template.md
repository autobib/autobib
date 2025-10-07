# Template syntax

*Templates* are used by Autobib to format record data.
This file documents the template syntax used by Autobib to format record data.

Record data is the information encapsulated in a record, such as

```bib
@article{zbmath:06346461,
  arxiv = {1212.1873},
  author = {Hochman, Michael},
  doi = {10.4007/annals.2014.180.2.7},
  journal = {Ann. Math.},
  month = {12},
  pages = {773--822},
  title = {On self-similar sets with overlaps and inverse theorems for entropy},
  volume = {180},
  year = {2014},
}
```

All examples in this document refer to this specific entry (unless otherwise noted).

## Syntax overview

### General syntax

A template is composed of *text* and *expressions*.
Expressions are delimited by curly braces: `{}`.
For example, the template

```txt
Hello {world}
```

consists of text `Hello ` followed by an expression `world`.

In order to include braces in text, duplicate the bracket, like `{{`.
In order to include braces inside expressions, use *extended delimiters*:  an opening bracket `{` can be followed by any number of `#` keys, and then it can only be closed by the same number of `#` keys, followed by `}`.
For example,

```txt
Hello {# "Brace {" #}
```

contains of text `Hello ` followed by an expression `"Brace {"`.

### Expression syntax

*Field keys* refer directly to the field values in the Bibtex data.
These are expressions like `author` or `title`, which expand into the corresponding keys:

```txt
'{author}: {title}' => 'Hochman, Michael: On self-similar sets with overlaps and inverse theorems for entropy'
```

The only permitted characters in a field key are ASCII letters and numbers, plus the underscore `_` character.
If your Bibtex entries contain other characters, you must manually escape them using brackets `()`.
For example, given data

```bib
@book{k,
  dots.and.qmark? = {Val},
}
```

we have the expansion

```txt
'{(docs.and.qmark?)}' => 'Val'
```

If a field key does not exist, the empty string is printed instead.
For example, since `editor` is not defined,

```txt
'{author}{editor}' => 'Hochman, Michael'
```

It is possible to [handle missed keys](#handling-missed-keys) to customize this behaviour.

There are also a number of *meta* expressions, which refer to metadata of the entry.
These are all prefixed by the `%` character:

- `%entry_type`: expands to the entry type, e.g. `article`.
- `%full_id`: expands to the full canonical id, e.g. `zbmath:06346461`.
- `%provider`: expands to provider of the canonical id: e.g. `zbmath`
- `%sub_id`: expands to provider of the canonical id: e.g. `06346461`

Finally, it is possible to input a *string*, i.e. a [JSON string](https://www.json.org/json-en.html), by quoting text.
This allows manually inputting invisible characters or specifying Unicode values using escapes by including the value in quotes:

```txt
'{"json \" string"}' => 'json " string'
```

Here, `\"` expands to `"` because JSON quotes must be escaped.

### Conditional expansion

In order to handle potentially missing keys, an expression can be prefixed by a *conditional* of the form `=key`, followed by whitespace and then any value from the previous section (that is, a *field key*, a *meta*, or a *string*).
For example:

```txt
'{author}{=subtitle ". "}{subtitle} => 'Hochman, Michael'
```

since the `subtitle` key is not defined.
On the other hand, since the `journal` key is defined,

```txt
'{author}{=journal ". "}{journal} => 'Hochman, Michael. Ann. Math.'
```

### Handling missed keys

Autobib commands which accept templates also accept a `-s/--strict` flag.
When used, this flag only formats if all of the field keys which would be rendered actually exist in the Bibtex data.

The precise behaviour depends on the command:

- `autobib find`: Any record missing a key is omitted from the search interface.

Strict mode prevents rendering if rendering the template would require expanding a field key which does not exist in the provided data.
For example:

1. Given a basic expression `{key}`, if `key` is not present.
2. Given a conditional expression `{=key1 key2}`, if `key1` is present and `key2` is not present.

The default behaviour can be enabled for specific field keys by appending a question mark: that is, writing `{key?}` in place of `{key}`.
Without strict mode, `{key?}` and `{key}` are equivalent.
The `{key?}` syntax is equivalent to the more repetitive `{=key key}`, and will also render (slightly) faster.

For example, in strict mode, the following expressions will fail to render:

- `{author}. {subtitle}`, since `subtitle` is not present.
- `{=title editor}`, since `title` is present but `editor` is not present

On the other hand,

```txt
'{author}{=editor subtitle}' => 'Hochman, Michael'
```

since `editor` is not defined, so `subtitle` does not need to be rendered.
