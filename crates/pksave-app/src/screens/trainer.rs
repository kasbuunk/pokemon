//! Trainer screen: names, money/coins, trainer ID, options, play time,
//! starters, safari steps and (Yellow) Pikachu friendship.

use pksave::gen1::save::GameVariant;
use pksave::gen1::trainer::{PlayTime, TextSpeed, MAX_COINS, MAX_MONEY};

use crate::app::Doc;
use crate::widgets;

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc) {
    ui.heading("Trainer");
    ui.add_space(4.0);
    let mut touched = false;

    egui::Grid::new("trainer_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Player name");
            if let Some(name) = widgets::name_edit(ui, "player_name", &doc.save.player_name()) {
                if doc.save.set_player_name(&name).is_ok() {
                    touched = true;
                }
            }
            ui.end_row();

            ui.label("Rival name");
            if let Some(name) = widgets::name_edit(ui, "rival_name", &doc.save.rival_name()) {
                if doc.save.set_rival_name(&name).is_ok() {
                    touched = true;
                }
            }
            ui.end_row();

            ui.label("Money");
            if let Some(v) = widgets::bcd_editor(ui, doc.save.money_lossy(), MAX_MONEY) {
                if doc.save.set_money(v).is_ok() {
                    touched = true;
                }
            }
            ui.end_row();

            ui.label("Coins");
            if let Some(v) = widgets::bcd_editor(ui, doc.save.coins_lossy(), MAX_COINS) {
                if doc.save.set_coins(v).is_ok() {
                    touched = true;
                }
            }
            ui.end_row();

            ui.label("Trainer ID");
            let mut id = doc.save.player_id();
            if widgets::word_stepper(ui, &mut id, 0..=u16::MAX) {
                doc.save.set_player_id(id);
                touched = true;
            }
            ui.end_row();

            ui.label("Safari steps");
            let mut steps = doc.save.safari_steps();
            if widgets::word_stepper(ui, &mut steps, 0..=u16::MAX) {
                doc.save.set_safari_steps(steps);
                touched = true;
            }
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.strong("Options");
        ui.horizontal(|ui| {
            ui.label("Text speed:");
            let current = doc.save.text_speed();
            for (speed, label) in [
                (TextSpeed::Fast, "Fast"),
                (TextSpeed::Medium, "Medium"),
                (TextSpeed::Slow, "Slow"),
            ] {
                if ui.radio(current == Some(speed), label).clicked() && current != Some(speed) {
                    doc.save.set_text_speed(speed);
                    touched = true;
                }
            }
            if current.is_none() {
                ui.weak(format!(
                    "(raw options nibble 0x{:X})",
                    doc.save.options() & 0x0F
                ));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Battle style:");
            let set_style = doc.save.battle_style_set();
            if ui.radio(!set_style, "Shift").clicked() && set_style {
                doc.save.set_battle_style_set(false);
                touched = true;
            }
            if ui.radio(set_style, "Set").clicked() && !set_style {
                doc.save.set_battle_style_set(true);
                touched = true;
            }
        });
        let mut animations_on = !doc.save.battle_animations_off();
        if ui
            .checkbox(&mut animations_on, "Battle animations")
            .changed()
        {
            doc.save.set_battle_animations_off(!animations_on);
            touched = true;
        }
    });

    if doc.variant == GameVariant::Yellow {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Pikachu friendship");
            let mut friendship = doc.save.pikachu_friendship();
            if widgets::byte_stepper(ui, &mut friendship, 0..=255) {
                doc.save.set_pikachu_friendship(friendship);
                touched = true;
            }
        });
    }

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.strong("Play time");
        let mut t = doc.save.play_time();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("H");
            changed |= widgets::byte_stepper(ui, &mut t.hours, 0..=255);
            ui.label("M");
            changed |= widgets::byte_stepper(ui, &mut t.minutes, 0..=59);
            ui.label("S");
            changed |= widgets::byte_stepper(ui, &mut t.seconds, 0..=59);
            ui.label("frames");
            changed |= widgets::byte_stepper(ui, &mut t.frames, 0..=59);
            changed |= ui.checkbox(&mut t.maxed, "clock maxed").changed();
        });
        if changed {
            doc.save.set_play_time(PlayTime { ..t });
            touched = true;
        }
    });

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.strong("Starters");
        ui.horizontal(|ui| {
            ui.label("Player:");
            if let Some(internal) =
                widgets::species_combo(ui, "player_starter", doc.save.player_starter())
            {
                doc.save.set_player_starter(internal);
                touched = true;
            }
            ui.label("Rival:");
            if let Some(internal) =
                widgets::species_combo(ui, "rival_starter", doc.save.rival_starter())
            {
                doc.save.set_rival_starter(internal);
                touched = true;
            }
        });
    });

    if touched {
        doc.touch();
    }
}
