//! Compiles the Slint user interface.

use slint_build::{CompilerConfiguration, EmbedResourcesKind};

fn main() -> Result<(), slint_build::CompileError> {
    // Fonts and icons ship inside the binary, whatever SLINT_EMBED_RESOURCES says.
    let config = CompilerConfiguration::new()
        .embed_resources(EmbedResourcesKind::EmbedFiles)
        .with_bundled_translations("po");
    slint_build::compile_with_config("ui/app.slint", config)
}
