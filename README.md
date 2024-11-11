# WARNING

Autobib is **alpha software**.

- Things might be broken.
- All of your data might be deleted.
- The interface may change without warning in unexpected ways.

On the other hand, Autobib is currently usable, and you are welcome to try it out with these caveats in mind.
Please report any issues in the [issues page](https://github.com/autobib/autobib/issues).

# Autobib

Autobib is a command-line tool for managing bibliographic records.
Unlike other bibliography management tools such as [Zotero](https://www.zotero.org/) or [JabRef](https://www.jabref.org/), Autobib aims to be a lower-level tool for providing an interface between *local records*, *remote records*, and *bibliographic records associated with a project*.

Moreover, Autobib is designed with first-class support for [BibTeX](https://en.wikipedia.org/wiki/BibTeX).

## Installation

Currently, the best way to install Autobib is to have the [rust toolchain](https://www.rust-lang.org/tools/install) installed on your device and run
```bash
cargo install --locked autobib
```

## Basic usage

To see all the commands available with Autobib, run
```bash
autobib help
autobib help <subcommand>
```

Jump to:

- [Getting records](#getting-records)
- [Sourcing from files](#sourcing-from-files)
- [Modifying records](#modifying-records)
- [Assigning aliases](#assigning-aliases)
- [Creating local records](#creating-local-records)
- [Searching for records](#searching-for-records)
- [Shell completions](#shell-completions)

### Getting records

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
  zbmath = {06346461},
}
```
An identifier is a pair `provider:sub_id`.
The currently supported providers are:

- `arxiv`: An [arXiv](https://arxiv.org) identifier, such as `arxiv:1212.1873` or `arxiv:math/9201254`
- `doi`: A [DOI](https://www.doi.org/) identifier, such as `doi:10.4007/annals.2014.180.2.7`
- `jfm`: A special [zbMath](https://zbmath.org) identifier mainly for old records, such as `jfm:60.0017.02`
- `mr`: A [MathSciNet](https://mathscinet.ams.org/mathscinet/publications-search) identifier, such as `mr:3224722`
- `zbl`: A [zbMath](https://zbmath.org) external identifier of the form `xxxx.xxxxx`, such as `zbl:1337.28015`
- `zbmath`: A [zbMath](https://zbmath.org) internal identifier of the form `xxxxxxxx`, such as `zbmath:06346461`

If your preferred format is not supported, feel free to [open an issue](https://github.com/autobib/autobib/issues) on the GitHub repository!

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

### Modifying records

On the first run, Autobib retrieves the data directly from a remote provider.
The data is stored locally in a [SQLite](https://www.sqlite.org/) database, which defaults to `~/.local/share/autobib/records.db`, so that subsequent runs are substantially faster.
You can view and modify the internally stored record with
```bash
autobib edit zbl:1337.28015
```
If the record does not yet exist in your local record database, it will be retrieved before editing.

You can also re-retrieve a record from the remote provider using the `autobib update` command, or remove one from the database using the `autobib delete` command.
Run `autobib help update` and `autobib help delete` for more details.

### Assigning aliases

It is also possible to assign *aliases* to records, using the `autobib alias` sub-command.
To create an alias for the record with identifier `zbl:1337.28015`, run
```bash
autobib alias add hochman-entropy zbl:1337.28015
```
Then running `autobib get hochman-entropy` returns
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
  zbmath = {06346461},
}
```
The record is identical to the record `zbl:1337.28015`, except that the citation key is the name of the alias.
In order to distinguish from usual identifiers, an alias cannot contain the colon `:`.

Note that the characters `{}(),=\#%"` and whitespaces are not permitted in a BibTeX entry key.
You can still create aliases using these characters: for instance, `autobib alias add % zbl:1337.28015` works.
However, attempting to retrieve the BibTeX entry associated with this alias will result in an error.
```
$ autobib get %
ERROR Invalid bibtex entry key: %
  Suggested fix: use an alias which does not contain disallowed characters: {}(),=\#%"
```
Run `autobib help alias` for more options for managing aliases.

Aliases can be used in most locations that the usual identifiers are used.
For instance, you can run `autobib edit hochman-entropy`, to edit the corresponding record data.
Note that these edits will apply to the original underlying record.

### Creating local records

Sometimes, it is necessary to create a local record which may not otherwise exist on a remote database.
In order to do this, the command `autobib local` can be used to generate a special `local:` record, which only exists locally in the database.
For example,
```bash
autobib local my-entry
```
creates a record under the identifier `local:my-entry`.
You will be prompted to fill in the record, unless you pass the `--no-edit` flag.
To modify the record later, use the [`autobib edit` command](#modifying-records).

It is also possible to create the local record from a BibTeX file:
```bash
autobib local my-entry --from source.bib --no-edit
```
Note that the BibTeX file should contain exactly one entry, or this command will fail.

When you create the local record `local:my-entry`, a new alias `my-entry` (if available) is also created and assigned to the new record.
As a consequence, the `sub_id` part of a `local:` identifier must be a valid alias, i.e. it cannot contain the colon `:`.

### Searching for records

In order to search for records which are saved on your local database, use the `autobib find` command.
This will open a fuzzy picker for searching through various fields.
Specify the fields that you would like with `-f`, followed by a comma-separated list of fields.
For example,
```bash
autobib find -f author,title
```
will list all of your local records with the `author` and `title` fields available to search against.

### Shell completions

Autobib supports shell completion of commands and options in shells like Bash and Zsh.
See [supported shells](https://docs.rs/clap_complete/latest/clap_complete/aot/enum.Shell.html).

A shell completions script can be generated as follows:
```sh
autobib completions <shell>
```
Run this on interactive shell start-up and redirect the output to your preferred directory of completions scripts.

For example, in Zsh, add the following lines to `~/.zshrc`:
```sh
if type autobib &> /dev/null
then
  autobib completions zsh > "$HOME/.local/share/zsh/site-functions/_autobib"
fi
```
and make sure `~/.local/share/zsh/site-functions` (or another directory of your choice) is added to the `FPATH` environment variable.

In Bash, add the following lines to `~/.profile`:
```sh
if type autobib &> /dev/null
then
  autobib completions bash > "$HOME/.local/share/bash-completion/completions/autobib"
fi
```

For this to take effect, remember to restart your shell or source the relevant file.
Then you can try typing `autobib f` and pressing the tab key.
You should see `autobib find `.

## License

The source code of Autobib is distributed under the terms of the [GNU Affero General Public License, version 3](https://www.gnu.org/licenses/agpl-3.0.en.html).

See [LICENSE](LICENSE) and [COPYRIGHT](COPYRIGHT) for details.
