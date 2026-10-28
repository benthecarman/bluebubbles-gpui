//! Icons for the app, with gpui-kit's own icons behind them for the
//! components that use those.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        /// The app's icons, drawn in the text colour.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Icon { $($name,)* }

        impl Icon {
            pub fn path(self) -> &'static str {
                match self { $(Icon::$name => concat!("bb/", $file, ".svg"),)* }
            }
        }

        fn embedded(path: &str) -> Option<&'static [u8]> {
            match path {
                "bb/app-icon.png" => Some(include_bytes!("../../../assets/icons/bluebubbles-256.png")),
                $(concat!("bb/", $file, ".svg") => Some(include_bytes!(concat!(
                    "../../../assets/icons/ui/", $file, ".svg"
                ))),)*
                _ => None,
            }
        }

        const PATHS: &[&str] = &[$(concat!("bb/", $file, ".svg"),)*];
    };
}

icons! {
    Compose => "compose",
    Search => "search",
    ArrowUp => "arrow-up",
    Person => "person",
    People => "people",
    Alert => "alert",
    Plus => "plus",
    Close => "xmark",
    Minimize => "minus",
    Maximize => "maximize",
    ChevronDown => "chevron-down",
    Offline => "wifi-off",
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match embedded(path) {
            Some(bytes) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out: Vec<SharedString> = PATHS
            .iter()
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(*p))
            .collect();
        out.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(out)
    }
}

/// An icon at `size`. Give it a `text_color`: SVGs don't inherit one.
pub fn icon(icon: Icon, size: gpui_kit::Pixels) -> gpui_kit::Svg {
    use gpui_kit::Styled;
    gpui_kit::svg().path(icon.path()).size(size).flex_none()
}
