# Using `autobib source`

If you have a file, say `main.tex`, you can run `autobib source main.tex` to generate a bibliography file from all of the citation keys which appear in `main.tex`.
The citation keys are sorted and deduplicated.

By default, the resulting bibliography is written to standard output.
You can instead write directly to a file with `--out`.
If you just want a list of the keys which were found, you can also use `--print-keys`.

## File-type detection

By default, Autobib tries to guess the format of your file based on the file name.
The following filetypes are supported:

- `.tex`, `.sty`: identifiers contained in `\cite`{...}` commands, and relatives.
- `.txt`: a single identifier per line
- `.aux`: the aux format `\abx@aux@cite{0}{...}`
- `.bib`: the bibtex identifiers

You can force the filetype behaviour with the `--file-type` flag.

## Standard input

It is also possible to search in standard input if you pass the `--stdin` flag.
By default, standard input is treated as a text file.

## Skipping existing keys

If you would like to exclude certain citation keys, you can pass them one at a time with the `--skip` option.
Any keys which appear verbatim in the input are automatically skipped.

You can also pass a file with `--skip-from`, which is equivalent to calling `--skip` for every identifier which can be found in the provided file.

## Appending to an existing file

If you have an existing bibtex file, you can use the `--append` flag to only write entries to the file which do not yet exist in a provided bibtex file.
This requires the `--out` argument to be specified, since that is the file to which the new keys will be added.
This is similar to running
```sh
autobib source main.tex --skip-from main.bib >> main.bib
```
