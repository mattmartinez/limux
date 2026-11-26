//! Palette for coloured sidebar workspace rows.
//!
//! Every colour is dark enough to carry white text. The palette, its
//! persisted names and the CSS that paints it all live here.

use serde::Deserialize;

/// CSS class on a sidebar row box that carries any palette colour.
pub const COLORED_ROW_CSS_CLASS: &str = "limux-ws-colored";
/// CSS class on the buttons that hold a swatch in the workspace context menu.
pub const SWATCH_BUTTON_CSS_CLASS: &str = "limux-ws-color-swatch-btn";
/// CSS class on the swatch label inside each of those buttons.
pub const SWATCH_CSS_CLASS: &str = "limux-ws-color-swatch";
/// CSS class on the swatch that clears the colour.
pub const NO_COLOR_SWATCH_CSS_CLASS: &str = "limux-ws-color-none";

#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceColor {
    Red,
    Orange,
    Amber,
    Green,
    Teal,
    Blue,
    Purple,
    Pink,
}

impl WorkspaceColor {
    pub const ALL: [WorkspaceColor; 8] = [
        Self::Red,
        Self::Orange,
        Self::Amber,
        Self::Green,
        Self::Teal,
        Self::Blue,
        Self::Purple,
        Self::Pink,
    ];

    /// Name used in the session file and on the control socket.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Amber => "amber",
            Self::Green => "green",
            Self::Teal => "teal",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Pink => "pink",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL
            .into_iter()
            .find(|color| color.as_str().eq_ignore_ascii_case(name))
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Amber => "Amber",
            Self::Green => "Green",
            Self::Teal => "Teal",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Pink => "Pink",
        }
    }

    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Red => (0xB9, 0x1C, 0x1C),
            Self::Orange => (0xAE, 0x3A, 0x0B),
            Self::Amber => (0x8A, 0x54, 0x06),
            Self::Green => (0x16, 0x6F, 0x37),
            Self::Teal => (0x0E, 0x6B, 0x64),
            Self::Blue => (0x1D, 0x4E, 0xD8),
            Self::Purple => (0x7E, 0x22, 0xCE),
            Self::Pink => (0xBE, 0x18, 0x5D),
        }
    }

    pub fn css_class(self) -> String {
        format!("limux-ws-color-{}", self.as_str())
    }

    /// The colour as a CSS hex value, for styles generated elsewhere.
    pub fn css_hex(self) -> String {
        hex(self.rgb())
    }
}

/// Comma-separated palette names, for error messages.
pub fn color_names() -> String {
    WorkspaceColor::ALL.map(WorkspaceColor::as_str).join(", ")
}

/// Deserialize an optional colour, mapping an unknown name to `None` so a
/// session written by a newer build still loads.
pub fn deserialize_option<'de, D>(deserializer: D) -> Result<Option<WorkspaceColor>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let name = Option::<String>::deserialize(deserializer)?;
    Ok(name.as_deref().and_then(WorkspaceColor::from_name))
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// cmux sidebar selection fill, sampled from the app (`#3E90F7`).
pub const SELECTION_FILL: &str = "#3e90f7";
/// cmux chrome black. The reference config sets `sidebarAppearance.tintColor` to `#000000`.
pub const CHROME_BLACK: &str = "#000000";

/// Mix a colour toward white by `percent`, for swatch hover.
fn lighten((r, g, b): (u8, u8, u8), percent: u16) -> (u8, u8, u8) {
    let mix = |channel: u8| {
        let channel = u16::from(channel);
        (channel + (255 - channel) * percent / 100) as u8
    };
    (mix(r), mix(g), mix(b))
}

const SHARED_CSS: &str = r#"
/* ---------- Coloured workspace rows ---------- */
/* The colour sits on a label inside the button: a theme installed as user
   CSS (~/.config/gtk-4.0/gtk.css) out-ranks application CSS for buttons. */
.limux-ws-color-swatch {
    min-width: 22px;
    min-height: 22px;
    padding: 0;
    border-radius: 999px;
    color: white;
    font-size: 11px;
}
.limux-ws-color-swatch.limux-ws-color-none {
    background: transparent;
    color: @window_fg_color;
    box-shadow: inset 0 0 0 1px alpha(@window_fg_color, 0.45);
}
button:hover > .limux-ws-color-swatch.limux-ws-color-none {
    background: alpha(@window_fg_color, 0.1);
}
"#;

/// Sizing of the swatch buttons. Loaded above user priority, because a theme
/// installed as user CSS otherwise pads every button and spreads the swatches.
pub const SWATCH_BUTTON_CSS: &str = r#"
button.limux-ws-color-swatch-btn {
    min-width: 0;
    min-height: 0;
    padding: 2px;
    margin: 0;
    border-radius: 999px;
}
"#;

/// CSS for coloured rows and the menu swatches, generated from the palette.
///
/// A workspace colour is a 3px left rail, the way cmux marks a card. The
/// selected row always uses [`SELECTION_FILL`], not the workspace colour.
pub fn workspace_color_css() -> String {
    let mut css = String::from(SHARED_CSS);
    for color in WorkspaceColor::ALL {
        let class = color.css_class();
        let base = hex(color.rgb());
        let hover = hex(lighten(color.rgb(), 12));
        // The colour class out-ranks the generic hover and selected backgrounds.
        css.push_str(&format!(
            ".limux-sidebar-list row .limux-sidebar-row-box.{class} {{\n\
                 background: transparent;\n\
                 box-shadow: inset 3px 0 0 0 {base};\n\
             }}\n\
             .limux-sidebar-list row:hover .limux-sidebar-row-box.{class} {{\n\
                 background: alpha(@window_fg_color, 0.06);\n\
                 box-shadow: inset 3px 0 0 0 {base};\n\
             }}\n\
             .limux-sidebar-list row:selected .limux-sidebar-row-box.{class},\n\
             .limux-sidebar-list row:selected:hover .limux-sidebar-row-box.{class} {{\n\
                 background: {SELECTION_FILL};\n\
                 box-shadow: none;\n\
             }}\n\
             .{SWATCH_CSS_CLASS}.{class} {{\n    background: {base};\n}}\n\
             button:hover > .{SWATCH_CSS_CLASS}.{class} {{\n    background: {hover};\n}}\n"
        ));
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    fn contrast_with_white(rgb: (u8, u8, u8)) -> f64 {
        1.05 / (relative_luminance(rgb) + 0.05)
    }

    #[test]
    fn palette_has_eight_distinct_colors() {
        let mut names: Vec<&str> = WorkspaceColor::ALL.map(WorkspaceColor::as_str).to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 8);
    }

    #[test]
    fn names_round_trip() {
        for color in WorkspaceColor::ALL {
            assert_eq!(WorkspaceColor::from_name(color.as_str()), Some(color));
            assert_eq!(
                serde_json::to_value(color).unwrap(),
                serde_json::json!(color.as_str())
            );
        }
        assert_eq!(
            WorkspaceColor::from_name(" Blue "),
            Some(WorkspaceColor::Blue)
        );
        assert_eq!(WorkspaceColor::from_name("chartreuse"), None);
        assert_eq!(WorkspaceColor::from_name(""), None);
    }

    #[test]
    fn every_color_is_readable_under_white_text() {
        for color in WorkspaceColor::ALL {
            // Swatch hover is the lightest a colour is painted under white text.
            for rgb in [color.rgb(), lighten(color.rgb(), 12)] {
                let contrast = contrast_with_white(rgb);
                assert!(
                    contrast >= 4.5,
                    "{} ({}) has contrast {contrast:.2} against white",
                    color.as_str(),
                    hex(rgb),
                );
            }
        }
    }

    #[test]
    fn rows_use_a_left_rail_and_the_standard_selection() {
        let css = workspace_color_css();
        assert!(css.contains(SELECTION_FILL));
        for color in WorkspaceColor::ALL {
            let class = color.css_class();
            let base = hex(color.rgb());
            assert!(
                css.contains(&format!("box-shadow: inset 3px 0 0 0 {base};")),
                "{class} is missing its left rail"
            );
            let selected =
                format!(".limux-sidebar-list row:selected .limux-sidebar-row-box.{class},");
            let rule = css
                .split(&selected)
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or("");
            assert!(
                rule.contains(SELECTION_FILL) && !rule.contains(&format!("background: {base}")),
                "{class} selected row does not use the standard selection fill: {rule}"
            );
        }
    }

    #[test]
    fn css_paints_every_color_for_rows_and_swatches() {
        let css = workspace_color_css();
        for color in WorkspaceColor::ALL {
            let class = color.css_class();
            assert!(css.contains(&format!(".limux-sidebar-row-box.{class}")));
            assert!(css.contains(&format!(".{SWATCH_CSS_CLASS}.{class} {{")));
            // Themes loaded as user CSS restyle buttons, so no swatch colour
            // may depend on a button background.
            assert!(!css.contains(&format!("button.{SWATCH_CSS_CLASS}.{class}")));
            assert!(css.contains(&hex(color.rgb())));
        }
        assert!(css.contains(&format!(
            ".{SWATCH_CSS_CLASS}.{NO_COLOR_SWATCH_CSS_CLASS} {{"
        )));
        assert!(SWATCH_BUTTON_CSS.contains(&format!("button.{SWATCH_BUTTON_CSS_CLASS} {{")));
    }
}
