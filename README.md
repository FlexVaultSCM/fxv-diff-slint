# fxv-diff-slint

A Slint widget for viewing text file diffs. It takes unified diff format as input and renders
it side by side or inline, with hunks separated by gaps that can be expanded on demand.

Targets desktop Windows, macOS, and Linux.

## Using it

```toml
[dependencies]
fxv-diff-slint = "0.1"
```

```slint
import { DiffView, DiffRow, DiffStyle } from "@FxvDiff";

export component MainWindow inherits Window {
    in property <[DiffRow]> rows;
    DiffView { rows: root.rows; }
}
```

No build script wiring is needed on your side. See `examples/viewer` for a working consumer.

## Building on Linux

Needs fontconfig and freetype development files: `fontconfig-devel` and `freetype-devel` on
Fedora, `libfontconfig-dev` and `libfreetype-dev` on Debian. Windows and macOS need nothing
extra.

## License

MIT. The bundled DejaVu Sans Mono font is under the Bitstream Vera license; see
`crates/fxv-diff-slint/fonts/LICENSE-DejaVu.txt`.
