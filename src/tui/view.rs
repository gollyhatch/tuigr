use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::library;
use crate::tui::app::{App, DetailRow, Mode, Pane, Row, StatusKind};
use crate::tui::help;

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.poll_cover();
    app.width = frame.area().width;
    app.height = frame.area().height;
    let area = frame.area();
    frame.render_widget(Block::default().style(app.theme.base()), area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, app, chunks[0]);
    draw_body(frame, app, chunks[1]);
    draw_status(frame, app, chunks[2]);
    draw_keybar(frame, app, chunks[3]);

    match app.mode {
        Mode::Edit => draw_editor(frame, app),
        Mode::Help => draw_help(frame, app),
        Mode::Cover => draw_cover_menu(frame, app),
        Mode::Lookup => draw_lookup(frame, app),
        Mode::ConfirmDelete => draw_confirm(frame, app),
        Mode::Theme => draw_theme_picker(frame, app),
        Mode::Browse | Mode::Search => {}
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let brand = app.theme.brand_line("tuigr");
    let counts = format!(
        "{} · {} · {}",
        count_label(app.artists.len(), "artist", "artists"),
        count_label(app.albums.len(), "album", "albums"),
        count_label(app.view.len(), "track", "tracks")
    );
    let mut left = vec![Span::raw(" ")];
    left.extend(brand.spans);
    left.push(Span::styled("  ›  ", app.theme.dim()));
    left.push(Span::styled(counts, app.theme.strong()));
    if !app.filter.is_empty() {
        left.push(Span::styled(
            format!("  /{}", truncate(&app.filter, 24)),
            app.theme.muted(),
        ));
    }
    if !app.selection.is_empty() {
        left.push(Span::styled(
            format!("  {} selected", app.selection.len()),
            app.theme.accent(),
        ));
    }

    let right_width = (area.width / 2).clamp(12, 48);
    let right_text = truncate(
        &format!("{}  ·  {} ", app.dir.display(), app.theme.display_name),
        right_width.saturating_sub(1) as usize,
    );
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(right_width)])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(left)).style(app.theme.base()),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(right_text, app.theme.muted())))
            .style(app.theme.base())
            .alignment(ratatui::layout::Alignment::Right),
        chunks[1],
    );
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let cover_width = if app.show_cover && area.width >= 110 {
        area.width * 40 / 100
    } else {
        0
    };
    if cover_width > 0 {
        let list_width = area.width * 30 / 100;
        let detail_width =
            area.width.saturating_sub(list_width + cover_width + 2);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(list_width),
                Constraint::Length(1),
                Constraint::Length(detail_width),
                Constraint::Length(1),
                Constraint::Length(cover_width),
            ])
            .split(area);
        draw_list(frame, app, chunks[0]);
        draw_detail(frame, app, chunks[2]);
        let inner_w = chunks[4].width.saturating_sub(2);
        let inner_h = chunks[4].height.saturating_sub(2);
        app.ensure_cover(inner_w, inner_h);
        draw_cover(frame, app, chunks[4]);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .spacing(1)
            .constraints([
                Constraint::Percentage(35),
                Constraint::Percentage(65),
            ])
            .split(area);
        draw_list(frame, app, chunks[0]);
        draw_detail(frame, app, chunks[1]);
    }
}

fn draw_list(frame: &mut Frame, app: &App, area: Rect) {
    let active = app.pane == Pane::List;
    let mut title = format!(" Library · {} ", app.sort.label());
    if !app.filter.is_empty() {
        title = " Library · filter ".to_string();
    }
    let block = pane_block(title, active, app, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = inner.height as usize;
    let mut lines = Vec::with_capacity(height);
    let start = app.list_top.min(app.rows.len());
    for (offset, row) in app.rows.iter().enumerate().skip(start).take(height) {
        let cursor = offset == app.list_index;
        lines.push(list_line(app, row, inner.width as usize, cursor, active));
    }
    frame.render_widget(Paragraph::new(lines).style(app.theme.base()), inner);
}

fn list_line(
    app: &App,
    row: &Row,
    width: usize,
    cursor: bool,
    active: bool,
) -> Line<'static> {
    let (text, mut style) = match row {
        Row::Artist(index) => {
            let artist = &app.artists[*index];
            let marker = if !app.filter.is_empty()
                || app.expanded_artists.contains(&artist.key)
            {
                "▾"
            } else {
                "▸"
            };
            let selected = artist_selected(app, artist);
            let check = if selected { "✓" } else { " " };
            (
                format!("{marker} {check} {}", artist.name),
                app.theme.artist(),
            )
        }
        Row::Album(index) => {
            let album = &app.albums[*index];
            let marker = if !app.filter.is_empty()
                || app.expanded_albums.contains(&album.key)
            {
                "▾"
            } else {
                "▸"
            };
            let selected = album_selected(app, album);
            let check = if selected { "✓" } else { " " };
            (
                format!("  {marker} {check} {}", album_row_label(album)),
                app.theme.album(),
            )
        }
        Row::Track { album, track } => {
            let entry = app.album_entry(*album, *track);
            let check = entry
                .map(|entry| app.selection.contains(&entry.path))
                .unwrap_or(false);
            let check = if check { "✓" } else { " " };
            let label = entry.map(|e| e.track_label()).unwrap_or_default();
            let title = entry.map(|e| e.title()).unwrap_or_default();
            let duration = entry
                .map(|e| library::duration_label(e.duration()))
                .unwrap_or_default();
            let label = if label.is_empty() { "--" } else { &label };
            let prefix = format!("     {check} {label:<4} ");
            let available =
                width.saturating_sub(prefix.width() + duration.width() + 1);
            let title = truncate(&title, available);
            let gap = width
                .saturating_sub(
                    prefix.width() + title.width() + duration.width(),
                )
                .max(1);
            (
                format!("{prefix}{title}{}{duration}", " ".repeat(gap)),
                app.theme.track(),
            )
        }
    };
    if cursor {
        style = if active {
            app.theme.selected()
        } else {
            app.theme.inactive_selection()
        };
    }
    Line::from(Span::styled(pad_or_truncate(&text, width), style))
}

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let active = app.pane == Pane::Detail;
    let title = detail_title(app);
    let block = pane_block(title, active, app, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = inner.height as usize;
    let mut lines = Vec::with_capacity(height);
    if app.details.is_empty() {
        lines.push(Line::from(Span::styled(
            "no track selected",
            app.theme.dim(),
        )));
    } else {
        let start = app.detail_top.min(app.details.len());
        for (offset, row) in
            app.details.iter().enumerate().skip(start).take(height)
        {
            let cursor = offset == app.detail_index;
            lines.push(detail_line(
                app,
                row,
                inner.width as usize,
                cursor,
                active,
            ));
        }
        if lines.len() < height {
            if let Some(entry) = app.current_entry() {
                let p = &entry.properties;
                let mut parts = vec![p.container.clone(), p.codec.clone()];
                parts.retain(|v| !v.is_empty());
                if p.bitrate_kbps > 0 {
                    parts.push(format!("{} kbps", p.bitrate_kbps));
                }
                if p.sample_rate > 0 {
                    parts.push(format!("{} Hz", p.sample_rate));
                }
                if p.bit_depth > 0 {
                    parts.push(format!("{} bit", p.bit_depth));
                }
                if p.channels > 0 {
                    parts.push(format!("{} ch", p.channels));
                }
                parts.push(library::duration_label(p.duration));
                parts.push(library::size_label(p.size));
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    truncate(&parts.join(" · "), inner.width as usize),
                    app.theme.help(),
                )));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines).style(app.theme.panel()), inner);
}

fn detail_line(
    app: &App,
    row: &DetailRow,
    width: usize,
    cursor: bool,
    active: bool,
) -> Line<'static> {
    let value = if row.mixed {
        "(mixed)".to_string()
    } else if row.value.is_empty() {
        "(none)".to_string()
    } else {
        row.value.clone()
    };
    let label_width = 16.min(width.saturating_sub(4));
    let label =
        format!("{:<width$}", format!("{}:", row.label), width = label_width);
    let available = width.saturating_sub(label_width + 1);
    let text = format!("{label} {}", truncate(&value, available));
    if cursor {
        let style = if active {
            app.theme.detail_cursor()
        } else {
            app.theme.inactive_selection()
        };
        Line::from(Span::styled(pad_or_truncate(&text, width), style))
    } else {
        let value_style = if row.read_only {
            app.theme.help()
        } else if row.mixed {
            app.theme.mixed()
        } else {
            app.theme.value()
        };
        Line::from(vec![
            Span::styled(label, app.theme.label()),
            Span::styled(
                format!(" {}", truncate(&value, available)),
                value_style,
            ),
        ])
    }
}

fn draw_cover(frame: &mut Frame, app: &App, area: Rect) {
    let block = pane_block(" Cover ".into(), false, app, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.cover_lines.is_empty() {
        let text = if app.cover_note.is_empty() {
            "no cover art"
        } else {
            &app.cover_note
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(text, app.theme.dim())))
                .style(app.theme.panel())
                .wrap(Wrap { trim: true }),
            inner,
        );
    } else {
        let lines: Vec<Line> = app
            .cover_lines
            .iter()
            .take(inner.height as usize)
            .cloned()
            .collect();
        frame.render_widget(
            Paragraph::new(lines).style(app.theme.panel()),
            inner,
        );
    }
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let line = if app.mode == Mode::Search {
        Line::from(vec![
            Span::styled(" / ", app.theme.key()),
            Span::styled(app.input.value.clone(), app.theme.text()),
            Span::styled("▏", app.theme.accent()),
        ])
    } else if !app.status.is_empty() {
        let style = match app.status_kind {
            StatusKind::Ok => app.theme.ok(),
            StatusKind::Error => app.theme.error(),
            StatusKind::Info => app.theme.info(),
        };
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                truncate(&app.status, area.width.saturating_sub(2) as usize),
                style,
            ),
        ])
    } else {
        let hint = app
            .current_entry()
            .map(|entry| entry.path.display().to_string())
            .unwrap_or_else(|| "Press ? for help".to_string());
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                truncate(&hint, area.width.saturating_sub(2) as usize),
                app.theme.dim(),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(line).style(app.theme.base()), area);
}

fn draw_keybar(frame: &mut Frame, app: &App, area: Rect) {
    let hints: &[(&str, &str)] = match app.mode {
        Mode::Search => &[
            ("Enter", "search"),
            ("Esc", "cancel"),
            ("ctrl+w", "delete word"),
        ],
        Mode::Edit => &[
            ("Enter", "save"),
            ("Esc", "cancel"),
            ("ctrl+w", "delete word"),
        ],
        Mode::Help => &[("j/k", "scroll"), ("q", "close")],
        Mode::Theme => {
            &[("j/k", "preview"), ("Enter", "apply"), ("Esc", "cancel")]
        }
        Mode::Cover => {
            if app.cover_editing {
                &[
                    ("Enter", "apply"),
                    ("Esc", "cancel"),
                    ("ctrl+w", "delete word"),
                ]
            } else {
                &[
                    ("d", "download"),
                    ("s", "set"),
                    ("x", "remove"),
                    ("e", "extract"),
                    ("Esc", "close"),
                ]
            }
        }
        Mode::Lookup => {
            &[("j/k", "choose"), ("Enter", "apply"), ("Esc", "close")]
        }
        Mode::ConfirmDelete => &[("y", "confirm"), ("n", "cancel")],
        Mode::Browse => {
            if app.pane == Pane::List {
                &[
                    ("j/k", "move"),
                    ("Enter", "expand"),
                    ("space", "select"),
                    ("/", "search"),
                    ("e", "edit"),
                    ("ctrl+t", "theme"),
                    ("?", "help"),
                    ("q", "quit"),
                ]
            } else {
                &[
                    ("j/k", "field"),
                    ("Enter", "edit"),
                    ("tab", "pane"),
                    ("ctrl+t", "theme"),
                    ("?", "help"),
                    ("q", "quit"),
                ]
            }
        }
    };

    let mut spans = vec![Span::raw(" ")];
    for (index, (key, label)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ", app.theme.dim()));
        }
        spans.push(Span::styled(*key, app.theme.key()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*label, app.theme.muted()));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(app.theme.panel()),
        area,
    );
}

fn draw_editor(frame: &mut Frame, app: &App) {
    let area = modal_area(frame.area(), 60, 9);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.help_border())
        .style(app.theme.panel())
        .title(Line::from(Span::styled(
            app.edit_row
                .as_ref()
                .map(|row| format!(" Edit {} ", row.label))
                .unwrap_or_else(|| " Edit ".into()),
            app.theme.editor_label(),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let context = if app.selection.len() >= 2 {
        "all selected tracks"
    } else if app.edit_row.as_ref().map(|r| r.album).unwrap_or(false) {
        "all tracks in the album"
    } else {
        "this track"
    };
    let input = input_line(app);
    let lines = vec![
        Line::from(Span::styled(
            format!("writing to {context}"),
            app.theme.editor_hint(),
        )),
        Line::default(),
        input,
        Line::default(),
        Line::from(Span::styled(
            "enter save · esc cancel",
            app.theme.editor_hint(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .style(app.theme.panel())
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn input_line(app: &App) -> Line<'static> {
    let chars: Vec<char> = app.input.value.chars().collect();
    let cursor = app.input.cursor.min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let after: String = chars[cursor..].iter().collect();
    let mut spans = vec![Span::raw(before)];
    if let Some(ch) = chars.get(cursor) {
        spans.push(Span::styled(
            ch.to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
        if after.chars().count() > 1 {
            spans.push(Span::raw(after.chars().skip(1).collect::<String>()));
        }
    } else {
        spans.push(Span::styled(
            " ",
            Style::default().add_modifier(Modifier::REVERSED),
        ));
    }
    Line::from(spans)
}

fn draw_help(frame: &mut Frame, app: &App) {
    let area = modal_area(frame.area(), 76, 34);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.help_border())
        .style(app.theme.panel())
        .title(Line::from(Span::styled(
            " tuigr help ",
            app.theme.editor_label(),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = help::help_lines();
    let height = inner.height as usize;
    let max_top = lines.len().saturating_sub(height);
    let top = app.help_top.min(max_top);
    let shown: Vec<Line> = lines
        .iter()
        .skip(top)
        .take(height.saturating_sub(1))
        .map(|line| {
            if line.is_empty() {
                Line::default()
            } else if !line.starts_with("  ") {
                Line::from(Span::styled(
                    line.clone(),
                    app.theme.pane_title_active(),
                ))
            } else {
                Line::from(Span::styled(line.clone(), app.theme.help()))
            }
        })
        .collect();
    let mut shown = shown;
    shown.push(Line::from(Span::styled(
        "↑↓ scroll · ? / esc / q close",
        app.theme.help(),
    )));
    frame.render_widget(Paragraph::new(shown).style(app.theme.panel()), inner);
}

fn draw_theme_picker(frame: &mut Frame, app: &App) {
    let area = modal_area(frame.area(), 54, 26);
    frame.render_widget(Clear, area);
    let title = if app.theme_choices.is_empty() {
        " Themes ".to_string()
    } else {
        format!(
            " Themes · {}/{} ",
            app.theme_index + 1,
            app.theme_choices.len()
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.help_border())
        .style(app.theme.panel())
        .title(Line::from(Span::styled(title, app.theme.editor_label())));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let footer = 2u16;
    let visible = inner.height.saturating_sub(footer) as usize;
    let mut lines = Vec::new();
    if app.theme_choices.is_empty() {
        lines.push(Line::from(Span::styled(
            "no themes available",
            app.theme.help(),
        )));
    } else {
        let top = if visible == 0 {
            0
        } else {
            app.theme_index.saturating_sub(visible - 1)
        };
        for (index, choice) in
            app.theme_choices.iter().enumerate().skip(top).take(visible)
        {
            let selected = index == app.theme_index;
            let style = if selected {
                app.theme.list_cursor()
            } else {
                app.theme.value()
            };
            let marker = if selected { "▸ " } else { "  " };
            let name_width = inner.width.saturating_sub(5) as usize;
            lines.push(Line::from(vec![
                Span::styled(marker, style),
                Span::styled(
                    "██ ",
                    Style::default().fg(choice.accent).bg(choice.background),
                ),
                Span::styled(pad_or_truncate(&choice.name, name_width), style),
            ]));
        }
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "↑↓/jk preview · enter apply · esc cancel",
        app.theme.help(),
    )));
    frame.render_widget(Paragraph::new(lines).style(app.theme.panel()), inner);
}

fn draw_cover_menu(frame: &mut Frame, app: &App) {
    let area = modal_area(frame.area(), 64, 13);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.help_border())
        .style(app.theme.panel())
        .title(Line::from(Span::styled(
            " Cover ",
            app.theme.editor_label(),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = Vec::new();
    if app.cover_editing {
        lines.push(Line::from(Span::styled("Cover file:", app.theme.label())));
        lines.push(input_line(app));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "enter apply · esc cancel",
            app.theme.editor_hint(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "d  download from MusicBrainz",
            app.theme.help(),
        )));
        lines.push(Line::from(Span::styled(
            "s  set from a JPEG/PNG file",
            app.theme.help(),
        )));
        lines.push(Line::from(Span::styled(
            "x  remove the front cover",
            app.theme.help(),
        )));
        lines.push(Line::from(Span::styled(
            "e  extract to the album folder",
            app.theme.help(),
        )));
        if let Some(entry) = app.current_entry() {
            if let Some(image) = entry.properties.images.first() {
                lines.push(Line::from(Span::styled(
                    format!(
                        "embedded: {} · {} · {} B",
                        image.kind, image.mime, image.size
                    ),
                    app.theme.help(),
                )));
            } else if let Some(folder) = &entry.properties.folder_cover {
                lines.push(Line::from(Span::styled(
                    format!("folder: {}", folder.display()),
                    app.theme.help(),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "no cover art",
                    app.theme.help(),
                )));
            }
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("esc close", app.theme.help())));
    }
    frame.render_widget(Paragraph::new(lines).style(app.theme.panel()), inner);
}

fn draw_lookup(frame: &mut Frame, app: &App) {
    let area = modal_area(frame.area(), 78, 20);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.help_border())
        .style(app.theme.panel())
        .title(Line::from(Span::styled(
            " MusicBrainz ",
            app.theme.editor_label(),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = Vec::new();
    for (index, release) in app.lookup_releases.iter().enumerate() {
        let line = format!(
            "{}{}. {} — {} ({})",
            if index == app.lookup_index {
                "▌ "
            } else {
                "  "
            },
            index + 1,
            release.artist_name(),
            release.title,
            release.date
        );
        let style = if index == app.lookup_index {
            app.theme.list_cursor()
        } else {
            app.theme.value()
        };
        lines.push(Line::from(Span::styled(
            pad_or_truncate(&line, inner.width as usize),
            style,
        )));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "↑↓ choose · enter apply album, titles, track/disc numbers and cover",
        app.theme.help(),
    )));
    lines.push(Line::from(Span::styled(
        "titles matched first, then tracks by order · esc close",
        app.theme.help(),
    )));
    frame.render_widget(Paragraph::new(lines).style(app.theme.panel()), inner);
}

fn draw_confirm(frame: &mut Frame, app: &App) {
    let area = modal_area(frame.area(), 56, 10);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.confirm_border())
        .style(app.theme.panel())
        .title(Line::from(Span::styled(
            format!(" Delete {} file(s)? ", app.delete_paths.len()),
            app.theme.error(),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines: Vec<Line> = app
        .delete_paths
        .iter()
        .take(4)
        .map(|path| {
            Line::from(Span::styled(
                truncate(
                    &path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    inner.width as usize,
                ),
                app.theme.value(),
            ))
        })
        .collect();
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "y/enter confirm · n/esc cancel",
        app.theme.help(),
    )));
    frame.render_widget(Paragraph::new(lines).style(app.theme.panel()), inner);
}

fn pane_block(
    title: String,
    active: bool,
    app: &App,
    panel: bool,
) -> Block<'static> {
    let style = if active {
        app.theme.pane_border_active()
    } else {
        app.theme.pane_border()
    };
    let title_style = if active {
        app.theme.pane_title_active()
    } else {
        app.theme.pane_title()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .style(if panel {
            app.theme.panel()
        } else {
            app.theme.base()
        })
        .title(Line::from(Span::styled(title, title_style)))
}

fn detail_title(app: &App) -> String {
    if app.selection.len() >= 2 {
        return format!(" Bulk · {} tracks ", app.selection.len());
    }
    match app.current_row() {
        Some(Row::Artist(index)) => app
            .artists
            .get(*index)
            .map(|artist| format!(" {} ", artist.name))
            .unwrap_or_else(|| " Detail ".into()),
        Some(Row::Album(index)) => app
            .albums
            .get(*index)
            .map(|album| format!(" {} ", album.label()))
            .unwrap_or_else(|| " Detail ".into()),
        Some(Row::Track { .. }) => app
            .current_entry()
            .map(|entry| {
                format!(
                    " {} ",
                    entry
                        .path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                )
            })
            .unwrap_or_else(|| " Detail ".into()),
        None => " Detail ".into(),
    }
}

fn album_row_label(album: &library::Album) -> String {
    let mut label = album.title.clone();
    if album.year > 0 {
        label.push_str(&format!(" ({})", album.year));
    }
    label
}

fn artist_selected(app: &App, artist: &library::ArtistGroup) -> bool {
    let entries: Vec<&crate::library::Entry> = artist
        .albums
        .iter()
        .filter_map(|index| app.albums.get(*index))
        .flat_map(|album| album.tracks.iter())
        .filter_map(|index| app.view.get(*index))
        .collect();
    !entries.is_empty()
        && entries
            .iter()
            .all(|entry| app.selection.contains(&entry.path))
}

fn album_selected(app: &App, album: &library::Album) -> bool {
    !album.tracks.is_empty()
        && album
            .tracks
            .iter()
            .filter_map(|index| app.view.get(*index))
            .all(|entry| app.selection.contains(&entry.path))
}

fn modal_area(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(5);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn truncate(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    for ch in value.chars() {
        if out.width() + ch.to_string().width() > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn pad_or_truncate(value: &str, width: usize) -> String {
    let value = truncate(value, width);
    let mut out = value;
    let current = out.width();
    if current < width {
        out.push_str(&" ".repeat(width - current));
    }
    out
}
