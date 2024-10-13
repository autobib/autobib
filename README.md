# WARNING
Autobib is **alpha software**.

- Things might be broken.
- All of your data might be deleted.
- The interface may change without warning in unexpected ways.

On the other hand, Autobib is currently usable, and you are welcome to try it out with these caveats in mind.
Please report any issues in the [issue page](https://github.com/autobib/autobib/issues).

# Autobib
Autobib is a command-line tool for managing bibliographic records.
Unlike other bibliography management tools such as [Zotero](https://www.zotero.org/) or [JabRef](https://www.jabref.org/), Autobib aims to be a lower-level tool for providing an interface between *local records*, *remote records*, and *bibliographic records associated with a project*.

Moreover, Autobib is designed with first-class support for [BibTeX](https://en.wikipedia.org/wiki/BibTeX).

## Installation
Currently, the best way to install Autobib is to have the [rust toolchain](https://www.rust-lang.org/tools/install) installed on your device and run
```bash
cargo install --git https://github.com/autobib/autobib.git
```

## Basic usage
In order to see all of the commands available to Autobib, run
```bash
autobib help
autobib help <subcommand>
```
Jump to:

- [Get records](#get-records)
- [Sourcing from files](#sourcing-from-files)
- [Edit records](#edit-records)
- [Assigning aliases](#assigning-aliases)
- [Creating local records](#creating-local-records)
- [Searching for records](#searching-for-records)

### Get records
At its most basic, Autobib converts *identifiers* into *records*.
To obtain the data associated with the zbMath record [`Zbl 1337.28015`](https://zbmath.org/1528.14024), running
```bash
autobib get zbl:1337.28015
```
will return
```bib
@article{zbl:1337.28015,
  author = {Hochman, Michael},
  doi = {10.4007/annals.2014.180.2.7},
  journal = {Ann. Math. (2)},
  language = {English},
  pages = {773--822},
  title = {On self-similar sets with overlaps and inverse theorems for entropy},
  volume = {180},
  year = {2014},
  zbl = {1337.28015},
}
```
An identifier is a pair `provider:sub_id`.
The current supported providers are:

- `arxiv`: An [arXiv](https://arxiv.org) identifier, such as `arxiv:1212.1873` or `arxiv:math/9201254`
- `doi`: A [DOI](https://www.doi.org/) identifier, such as `doi:10.4007/annals.2014.180.2.7`
- `jfm`: A special [zbMath](https://zbmath.org) identifier mainly for old records, such as `jfm:60.0017.02`
- `mr`: A [MathSciNet](https://mathscinet.ams.org/mathscinet/publications-search) identifier, such as `mr:3224722`
- `zbl`: A [zbMath](https://zbmath.org) external identifier of the form `xxxx.xxxxx`, such as `zbl:1337.28015`
- `zbmath`: A [zbMath](https://zbmath.org) internal identifier of the form `xxxxxxxx`, such as `zbmath:06346461`

### Sourcing from files
A more common scenario is that you have a file, say `main.tex`, with some contents:
```tex
% contents of file `main.tex`
\documentclass{article}
\begin{document}
We refer the reader to \cite{zbl:1337.28015,zbl:1409.11054}.
\end{document}
```
Then, for example, running
```bash
autobib source main.tex --out main.bib
```
will search through the document for valid citation keys and output the bibliography into the file `main.bib`.

### Edit records
On the first run, Autobib retrieves the data directly from a remote provider.
The data is stored locally in a [SQLite](https://www.sqlite.org/) database, which defaults to `~/.local/share/autobib/records.db`, so that subsequent runs are substantially faster.
You can view and update the internally stored record with
```bash
autobib edit zbl:1337.28015
```

### Assigning aliases
It is also possible to assign *aliases* to records, using the `autobib alias` sub-command.
For instance, run
```bash
autobib alias add hochman-entropy zbl:1337.28015
```
and then running `autobib get hochman-entropy` returns
```bib
@article{hochman-entropy,
  author = {Hochman, Michael},
  doi = {10.4007/annals.2014.180.2.7},
  journal = {Ann. Math. (2)},
  language = {English},
  pages = {773--822},
  title = {On self-similar sets with overlaps and inverse theorems for entropy},
  volume = {180},
  year = {2014},
  zbl = {1337.28015},
}
```
Note that the record is identical to the record `zbl:1337.28015`
In order to distinguish from usual identifiers, an alias cannot contain the `:` colon symbol.

Note that some characters are not permitted in BibTeX, more precisely the characters `{}(),=\#%"`.
You can add aliases using these characters: for instance, `autobib alias add % zbl:1337.28015`.
However, attempting to retrieve the BibTeX entry associated with this alias will result in an error.
```
$ autobib get %
ERROR Invalid bibtex entry key: %
  Suggested fix: use an alias which does not contain disallowed characters: {}(),=\#%"
```
Run `autobib help alias` for more options for managing aliases.

### Creating local records
Sometimes, it is necessary to create a local record which may not otherwise exist on a remote database.
In order to do this, the command `autobib local` can be used to generate a special `local:` record, which only exists locally database.
To modify the contents, run `autobib edit`.
For example:
```bash
autobib local my-entry
autobib edit local:my-entry
```

### Searching for records
In order to search for records which are saved on your local database, use the `autobib find` command.
This will open a fuzzy picker for searching through various fields.
Specify the fields that you would like with `-f`, followed by a comma-separated list of fields.
For example,
```bash
autobib find -f author,title
```
will list all of your local records with the `author` and `title` fields available to search against.

## License

Autobib is distributed under the terms of the [GNU Affero General Public License, version 3](https://www.gnu.org/licenses/agpl-3.0.en.html).

See [LICENSE](LICENSE) and [COPYRIGHT](COPYRIGHT) for details.
