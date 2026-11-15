//! Terminal user interface.

mod app;
mod coverart;
mod help;
mod input;
mod view;

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};

use crate::config::Config;

pub fn run(dir: &Path, config: Config) -> Result<()> {
    let app = app::App::new(dir, config);
    run_app(app)
}

pub fn run_file(path: &Path, config: Config) -> Result<()> {
    let app = app::App::new_file(path, config);
    run_app(app)
}

fn run_app(mut app: app::App) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|frame| view::draw(frame, &mut app))?;
            let timeout = if app.cover_loading() {
                Duration::from_millis(30)
            } else {
                Duration::from_millis(200)
            };
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        app.handle_key(key);
                    }
                }
            }
            if app.quit {
                break;
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn renders_library_and_detail() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("tuigr"));
        assert!(text.contains("Library"));
        assert!(text.contains("Unknown Artist"));
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use crate::config::Config;
    use crate::tags::{self, CoverAction, Field};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn artist_expands_to_albums() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        let before = app.rows.len();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.rows.len() > before);
        assert!(matches!(app.rows.get(1), Some(app::Row::Album(_))));
    }

    #[test]
    fn cover_panel_draws_artwork() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.flac");
        std::fs::copy("testdata/silence.flac", &path).unwrap();
        let data = tags::read(&path).unwrap();
        let mut png = Vec::new();
        let image =
            image::RgbaImage::from_pixel(4, 4, image::Rgba([220, 40, 40, 255]));
        image::DynamicImage::ImageRgba8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut png),
                image::ImageFormat::Png,
            )
            .unwrap();
        tags::write(&path, &data.metadata, CoverAction::Set(png)).unwrap();

        let mut app = app::App::new(dir.path(), Config::default());
        app.show_cover = true;
        let backend = TestBackend::new(140, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut text = String::new();
        for _ in 0..200 {
            terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
            text = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            if text.contains("▀") {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(text.contains("Cover"));
        assert!(
            text.contains("▀"),
            "lines={} note={:?}",
            app.cover_lines.len(),
            app.cover_note
        );
    }

    #[test]
    fn search_filters_tracks() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        app.filter = "missing:".into();
        app.rebuild();
        assert_eq!(app.view.len(), 5);
        let _ = app.search_url();
    }

    #[test]
    fn ctrl_w_deletes_search_word() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for ch in "foo bar".chars() {
            app.handle_key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            ));
        }
        assert_eq!(app.input.value, "foo bar");
        app.handle_key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(app.input.value, "foo ");
        assert_eq!(app.filter, "foo ");
    }

    #[test]
    fn theme_picker_previews_and_cancels() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        assert_eq!(app.theme_name, "ayu-dark");
        app.handle_key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(app.mode, app::Mode::Theme);
        assert_eq!(app.theme_name, "ayu-dark");
        assert!(app.theme_choices.len() > 1);

        let original = app.theme_name.clone();
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.mode, app::Mode::Theme);
        assert_ne!(app.theme_name, original);
        assert!(
            app.theme_choices
                .iter()
                .any(|choice| choice.id == app.theme_name)
        );

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, app::Mode::Browse);
        assert_eq!(app.theme_name, original);
    }

    #[test]
    fn theme_picker_applies_on_enter() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        app.handle_key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL,
        ));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let previewed = app.theme_name.clone();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, app::Mode::Browse);
        assert_eq!(app.theme_name, previewed);
    }

    #[test]
    fn theme_paints_canvas_and_panels() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let base = app.theme.color("bg.base");
        let panel = app.theme.color("bg.panel");
        assert_eq!(buffer.cell((0, 0)).unwrap().bg, base);
        assert_eq!(buffer.cell((5, 5)).unwrap().bg, base);
        assert_eq!(buffer.cell((60, 5)).unwrap().bg, panel);
        assert_eq!(buffer.cell((0, 39)).unwrap().bg, panel);
    }

    fn edit_track_field(field: Field, value: &str) {
        let dir = tempfile::tempdir().unwrap();
        for name in ["01.flac", "02.flac"] {
            std::fs::copy("testdata/silence.flac", dir.path().join(name))
                .unwrap();
        }
        let mut app = app::App::new(dir.path(), Config::default());
        let enter = || KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

        app.handle_key(enter()); // expand the artist
        app.move_list(1); // album row
        app.handle_key(enter()); // expand the album
        app.move_list(2); // first track
        app.handle_key(enter()); // focus the track editor
        assert_eq!(app.pane, app::Pane::Detail);

        let current = app.current_entry().unwrap().path.clone();
        let index = app
            .details
            .iter()
            .position(|row| row.field == Some(field))
            .unwrap_or_else(|| {
                panic!("track editor has no {} row", field.label())
            });
        assert!(
            !app.details[index].read_only,
            "{} is read-only",
            field.label()
        );
        app.detail_index = index;
        let before = app.details[index].value.clone();

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(app.mode, app::Mode::Edit);
        assert_eq!(app.input.value, before);
        for _ in 0..before.chars().count() {
            app.handle_key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::NONE,
            ));
        }
        assert_eq!(app.input.value, "");
        for ch in value.chars() {
            app.handle_key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            ));
        }
        app.handle_key(enter());
        assert_eq!(app.mode, app::Mode::Browse);

        let saved = tags::read(&current).unwrap();
        assert_eq!(saved.metadata.get(field), value, "{}", field.label());
        let untouched = if field == Field::Compilation {
            "no"
        } else {
            ""
        };
        for name in ["01.flac", "02.flac"] {
            let path = dir.path().join(name);
            if path == current {
                continue;
            }
            let other = tags::read(&path).unwrap();
            assert_eq!(
                other.metadata.get(field),
                untouched,
                "{} changed on {name}",
                field.label()
            );
        }
    }

    #[test]
    fn track_editor_sets_every_field_on_one_track() {
        edit_track_field(Field::Album, "Other Album");
        edit_track_field(Field::AlbumArtist, "Various Artists");
        edit_track_field(Field::Genre, "Ambient");
        edit_track_field(Field::Year, "1999");
        edit_track_field(Field::Publisher, "Label X");
        edit_track_field(Field::Compilation, "yes");
        edit_track_field(Field::Comment, "one off");
    }

    #[test]
    fn editors_put_common_fields_first() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        let enter = || KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

        app.handle_key(enter()); // expand the artist
        app.move_list(1); // album row
        let album: Vec<&str> =
            app.details.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            &album[..6],
            [
                "Album",
                "Album artist",
                "Genre",
                "Year",
                "Compilation",
                "Label"
            ]
        );

        app.handle_key(enter()); // expand the album
        app.move_list(1); // first track
        app.handle_key(enter()); // focus the track editor
        let track: Vec<&str> =
            app.details.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            &track[..8],
            [
                "Title",
                "Artist",
                "Album",
                "Album artist",
                "Genre",
                "Year",
                "Track",
                "Track total"
            ]
        );
        assert_eq!(track.last().copied(), Some("Lyrics"));
    }

    #[test]
    fn theme_picker_renders_choices() {
        let mut app = app::App::new(Path::new("testdata"), Config::default());
        app.handle_key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL,
        ));
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Themes"));
        assert!(text.contains("Ayu Dark"));
        assert!(text.contains("Dracula"));
        assert!(text.contains("preview"));
        let panel = app.theme.color("bg.panel");
        assert_eq!(
            terminal.backend().buffer().cell((40, 9)).unwrap().bg,
            panel
        );
    }
}
