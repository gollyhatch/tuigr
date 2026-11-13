pub const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Global",
        &[
            ("?", "open / close this help"),
            ("q / ctrl+c", "quit"),
            ("tab / shift+tab", "switch pane"),
            (
                "esc",
                "cancel editor / close dialog / clear selection or filter",
            ),
        ],
    ),
    (
        "Library",
        &[
            ("↑↓ / j k", "move"),
            ("home / end", "jump to top / bottom"),
            ("g / G", "jump to top / bottom"),
            ("ctrl+d / ctrl+u", "page down / up"),
            ("pgdn / pgup", "page down / up"),
            ("J / K", "next / previous artist"),
            ("→ / enter", "expand artist/album or focus detail"),
            ("l", "open the selected track in the detail pane"),
            ("h", "return from detail to the library"),
            ("←", "collapse artist or album"),
            ("E / C", "expand all / collapse all artists and albums"),
            ("space", "select track, album or artist"),
            ("ctrl+a", "select all visible tracks"),
            ("!", "jump to next track missing required tags"),
            ("/", "search"),
            ("S", "cycle album sort"),
            ("ctrl+t", "choose a color theme"),
            ("v", "toggle album cover panel"),
            ("R / ctrl+r", "rescan"),
            ("ctrl+k", "open the configured web search"),
            ("ctrl+o", "open the containing folder"),
        ],
    ),
    (
        "Actions",
        &[
            ("e / enter", "edit the field under the cursor"),
            ("d", "clear the field"),
            ("x", "toggle compilation"),
            ("T", "guess track numbers from file names"),
            ("c", "cover menu"),
            ("p / P", "play track / album"),
            ("m", "MusicBrainz lookup"),
            ("N / M", "rename / move to library"),
            ("D", "delete files"),
        ],
    ),
    (
        "Search",
        &[
            ("text", "type a query"),
            ("enter", "keep the filter and return to the library"),
            ("ctrl+w", "delete the previous word"),
            ("esc", "clear the filter"),
            ("ctrl+c", "quit"),
        ],
    ),
    (
        "Editor",
        &[
            ("enter", "save a field"),
            ("ctrl+w", "delete the previous word"),
            ("esc", "cancel editing"),
            ("← → / home / end", "move the text cursor"),
            ("ctrl+c", "quit"),
        ],
    ),
    (
        "Cover menu",
        &[
            ("d", "download from MusicBrainz"),
            ("s", "set from a JPEG/PNG file"),
            ("x", "remove the front cover"),
            ("e", "extract to the album folder"),
            ("ctrl+w", "delete the previous word when entering a file"),
            ("esc", "close the cover menu"),
        ],
    ),
    (
        "Theme picker",
        &[
            ("↑↓ / j k", "preview a theme"),
            ("enter", "apply the previewed theme"),
            ("esc / q", "cancel and restore the previous theme"),
        ],
    ),
    (
        "Delete confirmation",
        &[("y / enter", "confirm deletion"), ("n / esc", "cancel")],
    ),
    (
        "MusicBrainz dialog",
        &[
            ("↑↓ / j k", "choose a release"),
            ("enter", "apply the release"),
            ("esc", "close the dialog"),
        ],
    ),
];

pub fn help_lines() -> Vec<String> {
    let mut lines = Vec::new();
    for (section, keys) in HELP_SECTIONS {
        lines.push((*section).to_string());
        for (key, description) in *keys {
            lines.push(format!("  {:<18} {}", key, description));
        }
        lines.push(String::new());
    }
    lines
}
