# Working with attachments

Autobib can associate attachments with records.
Generally speaking, Autobib does not manage attachments for you, but rather expects that attachments are stored at specific locations, and provides a few basic utilities for working with attachments.

1. To determine the directory associated with an identifier, use `autobib path`.
2. Search for attachments using record metadata using `autobib find -m attachments`.
3. Add new attachments using `autobib attach`.

More complicated operations involving attachments should be done using a separate program.

## Attachment directory format and location

The attachment directory is determined by the first of the following which matches:

1. An explicit argument passed to `--attachments-dir`.
2. The value of `$AUTOBIB_ATTACHMENTS_DIR`.
3. The value of `$XDG_DATA_HOME/autobib/attachments`
4. `$HOME/.local/share/autobib/attachments`

The attachment directory format is currently subject to change and the subdirectory associated with an identifier should only be determined explicitly using `autobib path`.
The plan, eventually, is for the format to be stable, at which point it will be described precisely here.
