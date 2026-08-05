fn main() {
    // Nothing library-specific is needed here. The "@FxvDiff" import resolves through
    // metadata that fxv-diff-slint's build script emits.
    slint_build::compile("ui/viewer.slint").unwrap();
}
