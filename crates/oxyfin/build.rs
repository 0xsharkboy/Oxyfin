//! Compiles the Slint user interface.

fn main() -> Result<(), slint_build::CompileError> {
    slint_build::compile("ui/app.slint")
}
