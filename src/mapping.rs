// SPDX-License-Identifier: GPL-3.0-only

// Turn a COSMIC theme into Mattermost's custom theme JSON (all the keys it
// expects, as #rrggbb colors). A few COSMIC values can be translucent, so we
// composite them first.

use std::collections::BTreeMap;

use cosmic_theme::Theme;
use cosmic_theme::composite::over;
use cosmic_theme::palette::Srgba;
use tracing::warn;

use crate::config::{Overrides, ThemePrefs};

// Keys Mattermost's built-in themes set. If we omit some, the client fills gaps
// from its default palette, so we try to emit the full set.
pub const KNOWN_KEYS: &[&str] = &[
    "type",
    "sidebarBg",
    "sidebarText",
    "sidebarUnreadText",
    "sidebarTextHoverBg",
    "sidebarTextActiveBorder",
    "sidebarTextActiveColor",
    "sidebarHeaderBg",
    "sidebarHeaderTextColor",
    "sidebarTeamBarBg",
    "onlineIndicator",
    "awayIndicator",
    "dndIndicator",
    "mentionBg",
    "mentionBj",
    "mentionColor",
    "centerChannelBg",
    "centerChannelColor",
    "newMessageSeparator",
    "linkColor",
    "buttonBg",
    "buttonColor",
    "errorTextColor",
    "mentionHighlightBg",
    "mentionHighlightLink",
    "codeTheme",
];

pub type MattermostTheme = BTreeMap<String, String>;

/// Build the Mattermost theme for `theme`.
///
/// Translucent COSMIC colors are composited over their background so every
/// Mattermost color is an opaque `#rrggbb` value.
pub fn build(theme: &Theme, prefs: &ThemePrefs, overrides: &Overrides) -> MattermostTheme {
    let black = Srgba::new(0.0, 0.0, 0.0, 1.0);

    let bg = opaque(theme.bg_color(), black);
    let on_bg = opaque(theme.on_bg_color(), bg);

    let primary = opaque(theme.primary_container_color(), bg);
    let on_primary = opaque(theme.on_primary_container_color(), primary);
    let primary_component = opaque(theme.primary_component_color(), primary);

    let secondary = opaque(theme.secondary_container_color(), bg);
    let on_secondary = opaque(theme.on_secondary_container_color(), secondary);
    let secondary_component = opaque(theme.secondary_component_color(), secondary);

    let accent = opaque(theme.accent_color(), bg);
    let on_accent = opaque(theme.on_accent_color(), accent);
    let accent_text = opaque(theme.accent_text_color(), bg);

    let success = opaque(theme.success_color(), bg);
    let warning = opaque(theme.warning_color(), bg);
    let destructive = opaque(theme.destructive_color(), bg);

    let code_theme = if theme.is_dark {
        prefs.code_theme_dark.clone()
    } else {
        prefs.code_theme_light.clone()
    };

    let mut map = MattermostTheme::new();
    let mut put = |key: &str, value: String| {
        map.insert(key.to_owned(), value);
    };

    put("type", "custom".to_owned());
    put("sidebarBg", hex(primary));
    put("sidebarText", hex(on_primary));
    put("sidebarUnreadText", hex(on_primary));
    put("sidebarTextHoverBg", hex(primary_component));
    put("sidebarTextActiveBorder", hex(accent));
    put("sidebarTextActiveColor", hex(on_primary));
    put("sidebarHeaderBg", hex(secondary));
    put("sidebarHeaderTextColor", hex(on_secondary));
    put("sidebarTeamBarBg", hex(secondary_component));
    put("onlineIndicator", hex(success));
    put("awayIndicator", hex(warning));
    put("dndIndicator", hex(destructive));
    put("mentionBg", hex(accent));
    put("mentionBj", hex(accent));
    put("mentionColor", hex(on_accent));
    put("centerChannelBg", hex(bg));
    put("centerChannelColor", hex(on_bg));
    put("newMessageSeparator", hex(accent_text));
    put("linkColor", hex(accent_text));
    put("buttonBg", hex(accent));
    put("buttonColor", hex(on_accent));
    put("errorTextColor", hex(destructive));
    put("mentionHighlightBg", hex(accent));
    put("mentionHighlightLink", hex(on_accent));
    put("codeTheme", code_theme);

    for (key, value) in overrides.for_mode(theme.is_dark) {
        if KNOWN_KEYS.contains(&key.as_str()) {
            map.insert(key.clone(), value.clone());
        } else {
            warn!("ignoring override for unknown Mattermost theme key `{key}`");
        }
    }

    map
}

/// Composite `fg` over `bg`, returning an opaque color.
pub fn opaque(fg: Srgba, bg: Srgba) -> Srgba {
    let result = over(fg, bg);
    Srgba::new(result.red, result.green, result.blue, 1.0)
}

/// Format a color as lowercase `#rrggbb`.
pub fn hex(color: Srgba) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.red),
        channel(color.green),
        channel(color.blue)
    )
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_theme::ThemeBuilder;

    fn preferences() -> ThemePrefs {
        ThemePrefs::default()
    }

    #[test]
    fn dark_theme_contains_every_known_key() {
        let map = build(
            &ThemeBuilder::dark().build(),
            &preferences(),
            &Overrides::default(),
        );

        for key in KNOWN_KEYS {
            assert!(map.contains_key(*key), "missing theme key {key}");
        }
        assert_eq!(map["type"], "custom");
        assert_eq!(map["codeTheme"], "solarized-dark");
    }

    #[test]
    fn light_theme_uses_the_light_code_theme() {
        let map = build(
            &ThemeBuilder::light().build(),
            &preferences(),
            &Overrides::default(),
        );

        assert_eq!(map["codeTheme"], "github");
    }

    #[test]
    fn colors_are_opaque_lowercase_hex() {
        let map = build(
            &ThemeBuilder::dark().build(),
            &preferences(),
            &Overrides::default(),
        );

        for key in KNOWN_KEYS {
            if *key == "type" || *key == "codeTheme" {
                continue;
            }
            let value = &map[*key];
            assert_eq!(value.len(), 7, "{key} = {value}");
            assert!(value.starts_with('#'), "{key} = {value}");
            assert!(
                value[1..]
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{key} = {value}"
            );
        }
    }

    #[test]
    fn overrides_replace_generated_values() {
        let overrides = Overrides {
            dark: BTreeMap::from([("linkColor".to_owned(), "#ff0000".to_owned())]),
            light: BTreeMap::new(),
        };
        let map = build(&ThemeBuilder::dark().build(), &preferences(), &overrides);
        assert_eq!(map["linkColor"], "#ff0000");
    }

    #[test]
    fn unknown_overrides_are_ignored() {
        let overrides = Overrides {
            dark: BTreeMap::from([("notAKey".to_owned(), "#ff0000".to_owned())]),
            light: BTreeMap::new(),
        };
        let map = build(&ThemeBuilder::dark().build(), &preferences(), &overrides);
        assert!(!map.contains_key("notAKey"));
    }

    #[test]
    fn composites_translucent_colors() {
        // Mattermost requires opaque colors, so every translucent COSMIC color
        // is composited onto an already opaque background.
        let black = Srgba::new(0.0, 0.0, 0.0, 1.0);
        let white = Srgba::new(1.0, 1.0, 1.0, 1.0);
        let white_half = Srgba::new(1.0, 1.0, 1.0, 0.5);
        let black_half = Srgba::new(0.0, 0.0, 0.0, 0.5);
        assert_eq!(hex(opaque(white_half, black)), "#808080");
        assert_eq!(hex(opaque(black_half, white)), "#808080");
        assert_eq!(hex(opaque(white_half, white)), "#ffffff");
    }

    #[test]
    fn formats_hex_and_clamps() {
        assert_eq!(hex(Srgba::new(0.0, 0.0, 0.0, 1.0)), "#000000");
        assert_eq!(hex(Srgba::new(1.0, 1.0, 1.0, 1.0)), "#ffffff");
        assert_eq!(hex(Srgba::new(1.5, -1.0, 0.5, 1.0)), "#ff0080");
    }
}
