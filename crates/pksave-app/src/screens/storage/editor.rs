//! The "common record" editor shared by party, box and daycare records:
//! status, moves, DVs, stat exp, … — everything in the 33 shared bytes.
//! Edits are queued as [`MonEdit`] values against a borrowed-nothing
//! [`MonSnapshot`] and applied by [`apply_mon_edits`].

use pksave::gen1::data::{BASE_STATS, INDEX_TO_DEX, MOVES, TYPE_NAMES};
use pksave::gen1::pokemon::{
    MonMut, MonView, STATUS_BURNED, STATUS_FROZEN, STATUS_PARALYZED, STATUS_POISONED,
    STATUS_SLEEP_MASK,
};
use pksave::gen1::stats::{self, Dvs};

use crate::widgets;

/// The fields shared by party and box records, copied out of a view so
/// the editor borrows nothing.
#[derive(Clone, Copy)]
pub struct MonSnapshot {
    pub species: u8,
    pub current_hp: u16,
    pub box_level: u8,
    pub status: u8,
    pub types: (u8, u8),
    pub catch_rate: u8,
    pub moves: [u8; 4],
    pub ot_id: u16,
    pub exp: u32,
    pub stat_exps: [u16; 5],
    pub dvs: Dvs,
    pub pp: [u8; 4],
}

impl MonSnapshot {
    /// Copy the common fields out of any mon view — party, box and
    /// daycare records all share the [`MonView`] trait.
    pub fn read(mon: &impl MonView) -> MonSnapshot {
        MonSnapshot {
            species: mon.species(),
            current_hp: mon.current_hp(),
            box_level: mon.box_level(),
            status: mon.status(),
            types: mon.types(),
            catch_rate: mon.catch_rate(),
            moves: mon.moves(),
            ot_id: mon.ot_id(),
            exp: mon.exp(),
            stat_exps: mon.stat_exps(),
            dvs: mon.dvs(),
            pp: mon.pp(),
        }
    }

    /// The level the game derives from this record's experience — what
    /// a box mon withdraws at (see `MonView::level_from_exp`).
    pub fn level_from_exp(&self) -> u8 {
        let base = BASE_STATS[usize::from(INDEX_TO_DEX[usize::from(self.species)])];
        stats::level_for_exp(base.growth_rate, self.exp)
    }
}

/// An edit to a common record field, applied by [`apply_mon_edits`] to
/// any [`MonMut`] (party, box or daycare record).
#[derive(Clone, Copy)]
pub enum MonEdit {
    CurrentHp(u16),
    Status(u8),
    Types(u8, u8),
    CatchRate(u8),
    Moves([u8; 4]),
    Pp([u8; 4]),
    OtId(u16),
    StatExps([u16; 5]),
    Dvs(Dvs),
}

/// Apply the edits queued by [`common_editor`] to any mutable mon view.
pub fn apply_mon_edits(mon: &mut impl MonMut, edits: &[MonEdit]) {
    for edit in edits {
        match *edit {
            MonEdit::CurrentHp(v) => mon.set_current_hp(v),
            MonEdit::Status(v) => mon.set_status(v),
            MonEdit::Types(t1, t2) => mon.set_types(t1, t2),
            MonEdit::CatchRate(v) => mon.set_catch_rate(v),
            MonEdit::Moves(v) => mon.set_moves(v),
            MonEdit::Pp(v) => mon.set_pp(v),
            MonEdit::OtId(v) => mon.set_ot_id(v),
            MonEdit::StatExps(v) => mon.set_stat_exps(v),
            MonEdit::Dvs(v) => mon.set_dvs(v),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Healthy,
    Sleep,
    Poison,
    Burn,
    Freeze,
    Paralysis,
}

impl StatusKind {
    const ALL: [StatusKind; 6] = [
        StatusKind::Healthy,
        StatusKind::Sleep,
        StatusKind::Poison,
        StatusKind::Burn,
        StatusKind::Freeze,
        StatusKind::Paralysis,
    ];

    fn label(self) -> &'static str {
        match self {
            StatusKind::Healthy => "OK",
            StatusKind::Sleep => "Sleep",
            StatusKind::Poison => "Poison",
            StatusKind::Burn => "Burn",
            StatusKind::Freeze => "Freeze",
            StatusKind::Paralysis => "Paralysis",
        }
    }

    fn of(status: u8) -> StatusKind {
        if status & STATUS_SLEEP_MASK != 0 {
            StatusKind::Sleep
        } else if status & STATUS_POISONED != 0 {
            StatusKind::Poison
        } else if status & STATUS_BURNED != 0 {
            StatusKind::Burn
        } else if status & STATUS_FROZEN != 0 {
            StatusKind::Freeze
        } else if status & STATUS_PARALYZED != 0 {
            StatusKind::Paralysis
        } else {
            StatusKind::Healthy
        }
    }

    fn byte(self, sleep_turns: u8) -> u8 {
        match self {
            StatusKind::Healthy => 0,
            StatusKind::Sleep => sleep_turns.clamp(1, 7),
            StatusKind::Poison => STATUS_POISONED,
            StatusKind::Burn => STATUS_BURNED,
            StatusKind::Freeze => STATUS_FROZEN,
            StatusKind::Paralysis => STATUS_PARALYZED,
        }
    }
}

/// Picker over the valid Gen 1 type ids. Returns the new id, if changed.
fn type_combo(ui: &mut egui::Ui, id_salt: impl egui::AsIdSalt, current: u8) -> Option<u8> {
    let mut picked = None;
    egui::ComboBox::from_id_salt(ui.id().with(id_salt))
        .selected_text(widgets::type_label(current))
        .width(90.0)
        .show_ui(ui, |ui| {
            for (id, name) in TYPE_NAMES.iter().enumerate() {
                if name.is_empty() {
                    continue;
                }
                let id = id as u8;
                if ui.selectable_label(id == current, *name).clicked() && id != current {
                    picked = Some(id);
                }
            }
        });
    picked
}

/// Editor for the fields common to party and box records. Pushes the
/// requested changes into `edits`.
pub fn common_editor(
    ui: &mut egui::Ui,
    id_salt: &str,
    snap: &MonSnapshot,
    edits: &mut Vec<MonEdit>,
) {
    ui.horizontal(|ui| {
        ui.label("Types:");
        if let Some(t1) = type_combo(ui, (id_salt, "type1"), snap.types.0) {
            edits.push(MonEdit::Types(t1, snap.types.1));
        }
        if let Some(t2) = type_combo(ui, (id_salt, "type2"), snap.types.1) {
            edits.push(MonEdit::Types(snap.types.0, t2));
        }
        ui.label("Catch rate:");
        let mut catch_rate = snap.catch_rate;
        if widgets::byte_stepper(ui, &mut catch_rate, 0..=255) {
            edits.push(MonEdit::CatchRate(catch_rate));
        }
        ui.weak("(picking a species resets these)");
        ui.separator();
        ui.label("OT ID:");
        let mut ot_id = snap.ot_id;
        if widgets::word_stepper(ui, &mut ot_id, 0..=u16::MAX) {
            edits.push(MonEdit::OtId(ot_id));
        }
    });

    ui.horizontal(|ui| {
        ui.label("Status:");
        let kind = StatusKind::of(snap.status);
        let mut new_kind = kind;
        egui::ComboBox::from_id_salt(ui.id().with((id_salt, "status")))
            .selected_text(kind.label())
            .show_ui(ui, |ui| {
                for k in StatusKind::ALL {
                    ui.selectable_value(&mut new_kind, k, k.label());
                }
            });
        if new_kind != kind {
            edits.push(MonEdit::Status(new_kind.byte(1)));
        } else if kind == StatusKind::Sleep {
            let mut turns = snap.status & STATUS_SLEEP_MASK;
            ui.label("turns:");
            if widgets::byte_stepper(ui, &mut turns, 1..=7) {
                edits.push(MonEdit::Status(StatusKind::Sleep.byte(turns)));
            }
        }
        ui.separator();
        ui.label("Current HP:");
        let mut hp = snap.current_hp;
        if widgets::word_stepper(ui, &mut hp, 0..=999) {
            edits.push(MonEdit::CurrentHp(hp));
        }
    });

    ui.add_space(4.0);
    ui.strong("Moves");
    let move_options = widgets::move_options();
    for slot in 0..4 {
        ui.horizontal(|ui| {
            ui.monospace(format!("{}.", slot + 1));
            let current = snap.moves[slot];
            if let Some(new_move) = widgets::search_combo(
                ui,
                (id_salt, "move", slot),
                &widgets::move_label(current),
                &move_options,
            ) {
                let new_move = new_move as u8;
                let mut moves = snap.moves;
                let mut pp = snap.pp;
                moves[slot] = new_move;
                // A new move gets its full PP, preserving applied PP Ups.
                let max_pp = MOVES.get(usize::from(new_move)).map(|m| m.pp).unwrap_or(0);
                pp[slot] = stats::compose_pp(max_pp, stats::pp_ups(snap.pp[slot]));
                edits.push(MonEdit::Moves(moves));
                edits.push(MonEdit::Pp(pp));
            }
            let mut current_pp = stats::current_pp(snap.pp[slot]);
            let mut ups = stats::pp_ups(snap.pp[slot]);
            ui.label("PP");
            let pp_changed = widgets::byte_stepper(ui, &mut current_pp, 0..=63);
            ui.label("PP Up");
            let ups_changed = widgets::byte_stepper(ui, &mut ups, 0..=3);
            if pp_changed || ups_changed {
                let mut pp = snap.pp;
                pp[slot] = stats::compose_pp(current_pp, ups);
                edits.push(MonEdit::Pp(pp));
            }
        });
    }

    ui.add_space(4.0);
    ui.strong("DVs");
    {
        let mut dvs = snap.dvs;
        let mut changed = false;
        egui::Grid::new(ui.id().with((id_salt, "dvs")))
            .num_columns(4)
            .show(ui, |ui| {
                for (label, dv) in [
                    ("Attack", &mut dvs.attack),
                    ("Defense", &mut dvs.defense),
                    ("Speed", &mut dvs.speed),
                    ("Special", &mut dvs.special),
                ] {
                    ui.label(label);
                    changed |= ui.add(egui::Slider::new(dv, 0..=15)).changed();
                    ui.end_row();
                }
            });
        ui.weak(format!("Derived HP DV: {}", dvs.hp_dv()));
        if changed {
            edits.push(MonEdit::Dvs(dvs));
        }
    }

    ui.add_space(4.0);
    ui.strong("Stat experience");
    ui.horizontal(|ui| {
        let mut stat_exps = snap.stat_exps;
        let mut changed = false;
        for (label, se) in ["HP", "Atk", "Def", "Spd", "Spc"]
            .into_iter()
            .zip(stat_exps.iter_mut())
        {
            ui.label(label);
            changed |= widgets::word_stepper(ui, se, 0..=u16::MAX);
        }
        if changed {
            edits.push(MonEdit::StatExps(stat_exps));
        }
    });
}

/// What the Gen 1 formula yields for this record at `level`.
pub fn expected_stats(snap: &MonSnapshot, level: u8) -> [u16; 5] {
    let dex = INDEX_TO_DEX[usize::from(snap.species)];
    let base = BASE_STATS[usize::from(dex)];
    let se = snap.stat_exps;
    let dvs = snap.dvs;
    [
        stats::calc_stat(base.hp, dvs.hp_dv(), se[0], level, true),
        stats::calc_stat(base.attack, dvs.attack, se[1], level, false),
        stats::calc_stat(base.defense, dvs.defense, se[2], level, false),
        stats::calc_stat(base.speed, dvs.speed, se[3], level, false),
        stats::calc_stat(base.special, dvs.special, se[4], level, false),
    ]
}
