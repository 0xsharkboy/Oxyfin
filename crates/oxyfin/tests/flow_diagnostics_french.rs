//! Settings > Diagnostics in French: the action buttons keep their whole labels and the value
//! beside them wraps instead.

mod common;

use common::playback_settings::open_settings_prepared;
use common::wait_role;
use i_slint_backend_testing::AccessibleRole;
use reference::ButtonReference;
use slint::{ComponentHandle, LogicalSize};

#[allow(missing_docs)]
mod reference {
    slint::slint! {
        import "ui/fonts/Inter-Regular.ttf";
        import "ui/fonts/Inter-Medium.ttf";
        import "ui/fonts/Inter-SemiBold.ttf";
        import { Button, ButtonStyle } from "ui/components/button.slint";

        export component ButtonReference inherits Window {
            in property <string> label;
            out property <length> needed: button.preferred-width;
            default-font-family: "Inter";

            button := Button {
                label: root.label;
                style: ButtonStyle.secondary;
            }
        }
    }
}

// One app per test binary: the testing backend can be set up only once per process.
#[test]
fn french_action_buttons_keep_their_labels() {
    open_settings_prepared(
        Vec::new(),
        |_| {},
        |_| {},
        async move |window, _| {
            slint::select_bundled_translation("fr").map_err(|error| error.to_string())?;
            window.window().set_size(LogicalSize::new(1280., 2600.));
            let reference = ButtonReference::new().map_err(|error| error.to_string())?;
            for label in [
                "Vider le cache",
                "Copier les journaux",
                "Ouvrir le dossier",
                "Afficher",
            ] {
                let button = wait_role(window, label, AccessibleRole::Button).await?;
                reference.set_label(label.into());
                let needed = reference.get_needed();
                let width = button.size().width;
                if width + 0.5 < needed {
                    return Err(format!("{label} is {width} wide, cut from {needed}"));
                }
            }
            let value = wait_role(window, "Pour signaler un bug", AccessibleRole::Text).await?;
            let copy = wait_role(window, "Copier les journaux", AccessibleRole::Button).await?;
            let value_right = value.absolute_position().x + value.size().width;
            if value.size().width <= 0. || value_right > copy.absolute_position().x {
                return Err(format!(
                    "the value runs under the buttons: {:?} {:?}",
                    value.absolute_position(),
                    value.size()
                ));
            }
            Ok(())
        },
    );
}
