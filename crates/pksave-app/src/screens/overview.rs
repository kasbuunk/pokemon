//! Overview screen: player summary, party capsules and the diagnostics
//! table (click a span to inspect it in the hex view).

use egui_extras::{Column, TableBuilder};
use pksave::gen1::pokemon::{MonView, PartyMon};
use pksave::gen1::trainer::Badge;
use pksave::Severity;

use crate::app::{Doc, Screen};
use crate::widgets;

const BADGE_NAMES: [&str; 8] = [
    "Boulder", "Cascade", "Thunder", "Rainbow", "Soul", "Marsh", "Volcano", "Earth",
];

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, goto: &mut Option<(Screen, usize)>) {
    ui.heading("Overview");
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.strong(doc.save.player_name());
        ui.label(format!("(ID {:05})", doc.save.player_id()));
        ui.separator();
        ui.label(format!("₽{}", doc.save.money_lossy()));
        ui.separator();
        let t = doc.save.play_time();
        ui.label(format!(
            "{}:{:02}:{:02} played{}",
            t.hours,
            t.minutes,
            t.seconds,
            if t.maxed { " (maxed)" } else { "" }
        ));
    });

    ui.horizontal(|ui| {
        ui.label("Badges:");
        for (badge, name) in Badge::ALL.into_iter().zip(BADGE_NAMES) {
            let has = doc.save.has_badge(badge);
            let text = if has {
                egui::RichText::new(name).strong()
            } else {
                egui::RichText::new(name).weak().strikethrough()
            };
            ui.label(text);
        }
    });

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.strong("Party");
        let party = doc.save.party();
        if party.is_empty() {
            ui.weak("(empty)");
        }
        for i in 0..party.len() {
            let mon = party.mon(i);
            ui.horizontal(|ui| {
                ui.monospace(format!("{}.", i + 1));
                ui.label(party.nickname(i));
                ui.weak(widgets::species_label(mon.species()));
                ui.label(format!("Lv.{}", mon.level()));
                ui.weak(format!("{}/{} HP", mon.current_hp(), mon.max_hp()));
            });
        }
    });

    ui.add_space(8.0);
    ui.strong(format!("Diagnostics ({})", doc.diagnostics.len()));
    if doc.diagnostics.is_empty() {
        ui.weak("No findings — the file looks healthy.");
        return;
    }

    let diagnostics = doc.diagnostics.clone();
    TableBuilder::new(ui)
        .striped(true)
        .column(Column::auto())
        .column(Column::auto())
        .column(Column::remainder())
        .column(Column::auto())
        .header(20.0, |mut header| {
            header.col(|ui| {
                ui.strong("Sev");
            });
            header.col(|ui| {
                ui.strong("Code");
            });
            header.col(|ui| {
                ui.strong("Message");
            });
            header.col(|ui| {
                ui.strong("Span");
            });
        })
        .body(|mut body| {
            for diag in &diagnostics {
                body.row(20.0, |mut row| {
                    row.col(|ui| {
                        let (icon, color) = match diag.severity {
                            Severity::Info => ("ℹ", ui.visuals().weak_text_color()),
                            Severity::Warning => ("⚠", ui.visuals().warn_fg_color),
                            Severity::Error => ("⛔", ui.visuals().error_fg_color),
                        };
                        ui.colored_label(color, icon);
                    });
                    row.col(|ui| {
                        ui.monospace(diag.code);
                    });
                    row.col(|ui| {
                        ui.label(&diag.message);
                    });
                    row.col(|ui| {
                        if let Some(span) = &diag.span {
                            let text = format!("0x{:04X}..0x{:04X}", span.start, span.end);
                            if ui.link(text).on_hover_text("Show in hex view").clicked() {
                                *goto = Some((Screen::Hex, span.start));
                            }
                        } else {
                            ui.weak("—");
                        }
                    });
                });
            }
        });
}
