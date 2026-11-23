use crate::db::{self, Entry, Kind, Store};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use directories::ProjectDirs;
use eframe::egui::{self, Ui};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zeroize::Zeroize;

enum Screen {
    Setup,
    Unlock,
    Editor(Editor),
    Unavailable,
}

pub struct Journal {
    path: PathBuf,
    screen: Screen,
    encrypted: bool,
    password: String,
    confirmation: String,
    error: Option<String>,
}

impl Journal {
    pub fn new() -> Result<Self> {
        let dirs =
            ProjectDirs::from("", "", "Journal").context("Cannot locate your data directory")?;
        let path = dirs.data_local_dir().join("journal.db");
        let mut app = Self {
            path,
            screen: Screen::Unavailable,
            encrypted: true,
            password: String::new(),
            confirmation: String::new(),
            error: None,
        };
        app.inspect();
        Ok(app)
    }

    fn inspect(&mut self) {
        let result = db::inspect(&self.path).and_then(|kind| {
            Ok(match kind {
                Kind::Missing => Screen::Setup,
                Kind::Locked => Screen::Unlock,
                Kind::Plain => Screen::Editor(Editor::load(Store::open(&self.path, None)?)?),
            })
        });
        match result {
            Ok(screen) => {
                self.screen = screen;
                self.error = None;
            }
            Err(e) => {
                self.screen = Screen::Unavailable;
                self.error = Some(format!("{e:#}"));
            }
        }
    }

    fn gate(&mut self, ui: &mut Ui) {
        ui.heading("Journal");
        ui.add_space(12.0);
        if matches!(self.screen, Screen::Unavailable) {
            if ui.button("Retry opening journal").clicked() {
                self.inspect();
            }
            return;
        }
        let setup = matches!(self.screen, Screen::Setup);
        if setup {
            ui.label("Would you like to encrypt your new journal?");
            ui.radio_value(&mut self.encrypted, true, "Encrypt with a passphrase");
            ui.radio_value(&mut self.encrypted, false, "Store as plain SQLite");
        } else {
            ui.label("Unlock your journal");
        }
        let needs_key = !setup || self.encrypted;
        let mut ready = true;
        let mut unlock_with_enter = false;
        if needs_key {
            ui.label("Passphrase");
            let passphrase = ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
            unlock_with_enter = !setup
                && passphrase.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter));
            ready = !self.password.is_empty();
            if setup {
                ui.label("Confirm passphrase");
                ui.add(egui::TextEdit::singleline(&mut self.confirmation).password(true));
                ready &= self.password == self.confirmation;
                if !self.confirmation.is_empty() && self.password != self.confirmation {
                    ui.label("The passphrases differ.");
                }
                ui.label("Keep your passphrase safe. A lost passphrase cannot be recovered.");
            }
        } else {
            ui.label("Anyone who can read the database file can read your entries.");
            self.password.zeroize();
            self.confirmation.zeroize();
        }
        let label = if setup { "Create journal" } else { "Unlock" };
        let clicked = ui.add_enabled(ready, egui::Button::new(label)).clicked();
        if ready && (clicked || unlock_with_enter) {
            let key = needs_key.then_some(self.password.as_str());
            let result = if setup {
                Store::create(&self.path, key)
            } else {
                Store::open(&self.path, key)
            };
            match result.and_then(Editor::load) {
                Ok(editor) => {
                    self.screen = Screen::Editor(editor);
                    self.error = None;
                    self.password.zeroize();
                    self.confirmation.zeroize();
                }
                Err(e) => {
                    self.error = Some(format!("{e:#}"));
                }
            }
        }
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        self.password.zeroize();
        self.confirmation.zeroize();
    }
}

impl eframe::App for Journal {
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        if let Screen::Editor(editor) = &mut self.screen {
            if let Err(e) = editor.draw(ui) {
                self.error = Some(format!("{e:#}"));
            }
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                if let Err(e) = editor.flush() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.error = Some(format!("{e:#}"));
                }
            }
        } else {
            egui::CentralPanel::default().show(ui, |ui| self.gate(ui));
        }
        if let Some(message) = &self.error {
            let mut dismiss = false;
            egui::Window::new("Journal could not complete the action").show(ui.ctx(), |ui| {
                ui.label(message);
                dismiss = ui.button("Dismiss").clicked();
            });
            if dismiss {
                self.error = None;
            }
        }
    }
}

const SAVE_DELAY: Duration = Duration::from_millis(750);

struct Editor {
    store: Store,
    entries: Vec<Entry>,
    selected: Option<usize>,
    draft: String,
    save_due: Option<Instant>,
    confirm_delete: bool,
}

fn local_time(seconds: i64) -> String {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|t| {
            t.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| seconds.to_string())
}

impl Editor {
    fn load(store: Store) -> Result<Self> {
        let entries = store.entries()?;
        let mut editor = Self {
            store,
            entries,
            selected: None,
            draft: String::new(),
            save_due: None,
            confirm_delete: false,
        };
        editor.select_first();
        Ok(editor)
    }

    fn select_first(&mut self) {
        self.selected = (!self.entries.is_empty()).then_some(0);
        self.draft = self
            .entries
            .first()
            .map(|e| e.content.clone())
            .unwrap_or_default();
    }

    fn flush(&mut self) -> Result<()> {
        self.save_due = None;
        if let Some(index) = self.selected {
            let entry = &mut self.entries[index];
            if entry.content != self.draft {
                self.store.save(entry, &self.draft)?;
            }
        }
        Ok(())
    }

    fn draw(&mut self, ui: &mut Ui) -> Result<()> {
        let mut pick = None;
        let mut new = false;
        egui::Panel::right("entries")
            .default_size(245.0)
            .show(ui, |ui| {
                ui.heading("Entries");
                new = ui
                    .add_enabled(!self.confirm_delete, egui::Button::new("New entry"))
                    .clicked();
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, entry) in self.entries.iter().enumerate() {
                        let preview: String = entry
                            .content
                            .trim()
                            .lines()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .take(32)
                            .collect();
                        let label = format!("{}\n{}", local_time(entry.created), preview);
                        if ui
                            .selectable_label(self.selected == Some(i), label)
                            .clicked()
                        {
                            pick = Some(i);
                        }
                    }
                });
            });
        if !self.confirm_delete {
            if new {
                self.flush()?;
                let entry = self.store.insert()?;
                let index = self
                    .entries
                    .partition_point(|e| (e.created, e.id) > (entry.created, entry.id));
                self.entries.insert(index, entry);
                self.selected = Some(index);
                self.draft.clear();
            } else if let Some(index) = pick {
                self.flush()?;
                self.selected = Some(index);
                self.draft.clone_from(&self.entries[index].content);
            }
        }
        egui::CentralPanel::default()
            .show(ui, |ui| -> Result<()> {
                let Some(index) = self.selected else {
                    ui.label("Create an entry to start writing.");
                    return Ok(());
                };
                let entry = &self.entries[index];
                ui.label(format!("Created {}", local_time(entry.created)));
                ui.label(format!("Modified {}", local_time(entry.updated)));
                if self.confirm_delete {
                    ui.label("Permanently delete this entry?");
                    if ui.button("Delete permanently").clicked() {
                        self.store.delete(entry)?;
                        self.entries.remove(index);
                        self.select_first();
                        self.confirm_delete = false;
                        self.save_due = None;
                    } else if ui.button("Keep entry").clicked() {
                        self.confirm_delete = false;
                    }
                    return Ok(());
                }
                ui.horizontal(|ui| {
                    if ui.button("Delete entry…").clicked() {
                        self.confirm_delete = true;
                    }
                    if self.draft != entry.content {
                        ui.label("Unsaved changes");
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("editor", entry.id))
                    .show(ui, |ui| {
                        let edit = egui::TextEdit::multiline(&mut self.draft)
                            .id_salt(("text", entry.id))
                            .desired_width(f32::INFINITY)
                            .desired_rows(18);
                        if ui.add(edit).changed() {
                            self.save_due = Some(Instant::now() + SAVE_DELAY);
                        }
                    });
                Ok(())
            })
            .inner?;
        if let Some(due) = self.save_due {
            let remaining = due.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.flush()?;
            } else {
                ui.ctx().request_repaint_after(remaining);
            }
        }
        Ok(())
    }
}
