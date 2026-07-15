//! Party screen: the 6 slots plus a full detail editor for the selected
//! mon, and the daycare. The "common record" editor (status, moves, DVs,
//! stat exp, …) is shared with the box screen via [`MonSnapshot`] /
//! [`MonEdit`].

use pksave::gen1::data::{BASE_STATS, INDEX_TO_DEX, MOVES, SPECIES_NAMES, TYPE_NAMES};
use pksave::gen1::pokemon::{
    box_to_party, party_to_box, BoxMonView, MonMut, MonView, PartyMon, PartyMonMut, STATUS_BURNED,
    STATUS_FROZEN, STATUS_PARALYZED, STATUS_POISONED, STATUS_SLEEP_MASK,
};
use pksave::gen1::stats::{self, Dvs};
use pksave::gen1::{offsets, text};

use crate::app::Doc;
use crate::widgets;

#[derive(Default)]
pub struct PartyState {
    pub selected: usize,
    /// Internal species index picked in the "add" row.
    pub add_species: u8,
}

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

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, state: &mut PartyState) {
    egui::ScrollArea::vertical()
        .id_salt("party_screen")
        .show(ui, |ui| {
            party_body(ui, doc, state);
        });
}

fn party_body(ui: &mut egui::Ui, doc: &mut Doc, state: &mut PartyState) {
    ui.heading("Party");
    ui.add_space(4.0);
    let mut touched = false;

    let party_len = doc.save.party().len();
    state.selected = state.selected.min(party_len.saturating_sub(1));

    // ---- slot list + add/remove/reorder ----
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(230.0);
            for i in 0..party_len {
                let (nick, label) = {
                    let party = doc.save.party();
                    let mon = party.mon(i);
                    (
                        party.nickname(i),
                        format!(
                            "Lv.{} {}",
                            mon.level(),
                            widgets::species_label(mon.species())
                        ),
                    )
                };
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(state.selected == i, format!("{nick} — {label}"))
                        .clicked()
                    {
                        state.selected = i;
                    }
                });
            }
            if party_len == 0 {
                ui.weak("(empty party)");
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let can_up = state.selected > 0 && party_len > 1;
                if ui.add_enabled(can_up, egui::Button::new("⬆ Up")).clicked() {
                    doc.save
                        .party_mut()
                        .swap(state.selected, state.selected - 1);
                    state.selected -= 1;
                    touched = true;
                }
                let can_down = party_len > 1 && state.selected + 1 < party_len;
                if ui
                    .add_enabled(can_down, egui::Button::new("⬇ Down"))
                    .clicked()
                {
                    doc.save
                        .party_mut()
                        .swap(state.selected, state.selected + 1);
                    state.selected += 1;
                    touched = true;
                }
                if ui
                    .add_enabled(party_len > 0, egui::Button::new("🗑 Remove"))
                    .clicked()
                {
                    doc.save.party_mut().remove(state.selected);
                    touched = true;
                }
            });
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.strong("Add Pokémon");
                if state.add_species == 0 {
                    state.add_species = 0x99; // Bulbasaur
                }
                if let Some(picked) = widgets::species_combo(ui, "add_species", state.add_species) {
                    state.add_species = picked;
                }
                let full = party_len >= offsets::PARTY_CAPACITY;
                if ui
                    .add_enabled(!full, egui::Button::new("Add at level 5"))
                    .clicked()
                    && add_mon(doc, state.add_species)
                {
                    state.selected = doc.save.party().len().saturating_sub(1);
                    touched = true;
                }
                ui.weak("Added with zero DVs and no moves — set moves below.");
            });
        });

        ui.separator();

        // ---- detail editor ----
        ui.vertical(|ui| {
            if party_len == 0 {
                ui.weak("Add a Pokémon to edit it.");
                return;
            }
            let i = state.selected;
            touched |= detail_editor(ui, doc, i);
        });
    });

    // ---- daycare ----
    ui.add_space(12.0);
    egui::CollapsingHeader::new("Daycare")
        .default_open(doc.save.daycare().is_some())
        .show(ui, |ui| {
            touched |= daycare_section(ui, doc);
        });

    if touched {
        doc.touch();
    }
}

/// Build a minimal legal level-5 mon of `internal` species and append it.
fn add_mon(doc: &mut Doc, internal: u8) -> bool {
    let dex = INDEX_TO_DEX[usize::from(internal)];
    let base = BASE_STATS[usize::from(dex)];
    let mut record = [0u8; offsets::PARTY_MON_SIZE];
    {
        let mut mon = PartyMonMut::new(&mut record);
        mon.set_species(internal);
        mon.set_types(base.type1, base.type2);
        mon.set_catch_rate(base.catch_rate);
        mon.set_ot_id(doc.save.player_id());
        mon.set_level_coherent(5);
    }
    let nickname = if dex == 0 {
        "MISSINGNO.".to_owned()
    } else {
        SPECIES_NAMES[usize::from(dex)].to_owned()
    };
    let ot = player_ot_name(doc);
    doc.save.party_mut().add(&record, &ot, &nickname).is_ok()
}

/// The player's name if it encodes, else a safe fallback OT.
fn player_ot_name(doc: &Doc) -> String {
    let ot = doc.save.player_name();
    if text::encode(&ot, offsets::NAME_LEN).is_ok() {
        ot
    } else {
        "TRAINER".to_owned()
    }
}

/// The per-slot detail editor. Returns whether anything changed.
fn detail_editor(ui: &mut egui::Ui, doc: &mut Doc, i: usize) -> bool {
    let mut touched = false;
    let snap = MonSnapshot::read(&doc.save.party().mon(i));
    let (nickname, ot_name, level, stats_now) = {
        let party = doc.save.party();
        let mon = party.mon(i);
        (
            party.nickname(i),
            party.ot_name(i),
            mon.level(),
            [
                mon.max_hp(),
                mon.attack(),
                mon.defense(),
                mon.speed(),
                mon.special(),
            ],
        )
    };
    let base = BASE_STATS[usize::from(INDEX_TO_DEX[usize::from(snap.species)])];

    // ---- identity ----
    ui.horizontal(|ui| {
        ui.label("Species:");
        if let Some(internal) = widgets::species_combo(ui, ("party_species", i), snap.species) {
            let new_dex = INDEX_TO_DEX[usize::from(internal)];
            let new_base = BASE_STATS[usize::from(new_dex)];
            let mut party = doc.save.party_mut();
            party.set_species(i, internal);
            let mut mon = party.mon_mut(i);
            mon.set_types(new_base.type1, new_base.type2);
            mon.set_catch_rate(new_base.catch_rate);
            touched = true;
        }
        ui.label("Nickname:");
        if let Some(name) = widgets::name_edit(ui, ("party_nick", i), &nickname) {
            if doc.save.party_mut().set_nickname(i, &name).is_ok() {
                touched = true;
            }
        }
        ui.label("OT:");
        if let Some(name) = widgets::name_edit(ui, ("party_ot", i), &ot_name) {
            if doc.save.party_mut().set_ot_name(i, &name).is_ok() {
                touched = true;
            }
        }
    });

    // ---- level & exp ----
    ui.horizontal(|ui| {
        ui.label("Level:");
        let mut new_level = level;
        if widgets::byte_stepper(ui, &mut new_level, 1..=100) {
            doc.save
                .party_mut()
                .mon_mut(i)
                .set_level_coherent(new_level);
            touched = true;
        }
        ui.weak("(sets exp & stats coherently, full-heals)");
    });
    ui.horizontal(|ui| {
        ui.label("Exp:");
        let mut exp = snap.exp;
        if ui
            .add(
                egui::DragValue::new(&mut exp)
                    .range(0..=0x00FF_FFFFu32)
                    .speed(100),
            )
            .changed()
        {
            doc.save.party_mut().mon_mut(i).set_exp(exp);
            touched = true;
        }
        if ui
            .button("Sync from level")
            .on_hover_text("exp := exp_for_level(growth curve, level)")
            .clicked()
        {
            let exp = stats::exp_for_level(base.growth_rate, level);
            doc.save.party_mut().mon_mut(i).set_exp(exp);
            touched = true;
        }
        ui.weak(format!(
            "(level for exp: {})",
            stats::level_for_exp(base.growth_rate, snap.exp)
        ));
    });

    // ---- common record fields ----
    let mut edits = Vec::new();
    common_editor(ui, "party", &snap, &mut edits);
    if !edits.is_empty() {
        let mut party = doc.save.party_mut();
        let mut mon = party.mon_mut(i);
        apply_mon_edits(&mut mon, &edits);
        touched = true;
    }

    // ---- calculated stats ----
    ui.add_space(4.0);
    ui.strong("Calculated stats");
    ui.horizontal(|ui| {
        let mut stored = stats_now;
        let mut changed = false;
        for (label, value) in ["Max HP", "Atk", "Def", "Spd", "Spc"]
            .into_iter()
            .zip(stored.iter_mut())
        {
            ui.label(label);
            changed |= widgets::word_stepper(ui, value, 0..=999);
        }
        if changed {
            let mut party = doc.save.party_mut();
            let mut mon = party.mon_mut(i);
            mon.set_max_hp(stored[0]);
            mon.set_attack(stored[1]);
            mon.set_defense(stored[2]);
            mon.set_speed(stored[3]);
            mon.set_special(stored[4]);
            touched = true;
        }
    });
    ui.horizontal(|ui| {
        let expected = expected_stats(&snap, level);
        ui.weak(format!(
            "From formula: {} / {} / {} / {} / {}",
            expected[0], expected[1], expected[2], expected[3], expected[4]
        ));
        if ui
            .button("Recalculate")
            .on_hover_text("Recompute the five stats from base stats, DVs and stat exp")
            .clicked()
        {
            doc.save.party_mut().mon_mut(i).recalculate_stats();
            touched = true;
        }
    });

    touched
}

/// The daycare occupant editor (box-format record, nickname, OT) plus
/// deposit-from-party / take-to-party. Returns whether anything changed.
fn daycare_section(ui: &mut egui::Ui, doc: &mut Doc) -> bool {
    let mut touched = false;
    let occupant = doc.save.daycare().map(|view| {
        let mut record = [0u8; offsets::BOX_MON_SIZE];
        record.copy_from_slice(view.mon().as_bytes());
        (record, view.nickname(), view.ot_name())
    });

    let Some((record, nickname, ot_name)) = occupant else {
        ui.weak("The daycare is empty.");
        let party_len = doc.save.party().len();
        if party_len == 0 {
            return false;
        }
        ui.add_space(4.0);
        ui.strong("Deposit from party");
        for p in 0..party_len {
            let (nick, ot, label, party_record) = {
                let party = doc.save.party();
                let mon = party.mon(p);
                let mut rec = [0u8; offsets::PARTY_MON_SIZE];
                rec.copy_from_slice(mon.as_bytes());
                (
                    party.nickname(p),
                    party.ot_name(p),
                    format!(
                        "Lv.{} {}",
                        mon.level(),
                        widgets::species_label(mon.species())
                    ),
                    rec,
                )
            };
            let mut deposited = false;
            ui.horizontal(|ui| {
                ui.label(format!("{nick} — {label}"));
                if ui.button("Deposit").clicked() {
                    // The level moves into the box level byte, as in-game.
                    let box_record = party_to_box(&party_record);
                    if doc
                        .save
                        .set_daycare(Some((&box_record, &ot, &nick)))
                        .is_ok()
                    {
                        doc.save.party_mut().remove(p);
                        deposited = true;
                        touched = true;
                    }
                }
            });
            if deposited {
                break;
            }
        }
        return touched;
    };

    let snap = MonSnapshot::read(&BoxMonView::new(&record));

    ui.horizontal(|ui| {
        ui.label("Species:");
        if let Some(internal) = widgets::species_combo(ui, "daycare_species", snap.species) {
            let new_base = BASE_STATS[usize::from(INDEX_TO_DEX[usize::from(internal)])];
            if let Some(mut daycare) = doc.save.daycare_mut() {
                let mut mon = daycare.mon_mut();
                mon.set_species(internal);
                mon.set_types(new_base.type1, new_base.type2);
                mon.set_catch_rate(new_base.catch_rate);
                touched = true;
            }
        }
        ui.label("Nickname:");
        if let Some(name) = widgets::name_edit(ui, "daycare_nick", &nickname) {
            if let Some(mut daycare) = doc.save.daycare_mut() {
                if daycare.set_nickname(&name).is_ok() {
                    touched = true;
                }
            }
        }
        ui.label("OT:");
        if let Some(name) = widgets::name_edit(ui, "daycare_ot", &ot_name) {
            if let Some(mut daycare) = doc.save.daycare_mut() {
                if daycare.set_ot_name(&name).is_ok() {
                    touched = true;
                }
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("Level (box):");
        let mut level = snap.box_level;
        if widgets::byte_stepper(ui, &mut level, 1..=100) {
            if let Some(mut daycare) = doc.save.daycare_mut() {
                daycare.mon_mut().set_box_level(level);
                touched = true;
            }
        }
        ui.separator();
        ui.label("Exp:");
        let mut exp = snap.exp;
        if ui
            .add(
                egui::DragValue::new(&mut exp)
                    .range(0..=0x00FF_FFFFu32)
                    .speed(100),
            )
            .changed()
        {
            if let Some(mut daycare) = doc.save.daycare_mut() {
                daycare.mon_mut().set_exp(exp);
                touched = true;
            }
        }
        ui.weak("Stats are recomputed when it re-joins the party.");
    });

    let mut edits = Vec::new();
    common_editor(ui, "daycare", &snap, &mut edits);
    if !edits.is_empty() {
        if let Some(mut daycare) = doc.save.daycare_mut() {
            let mut mon = daycare.mon_mut();
            apply_mon_edits(&mut mon, &edits);
            touched = true;
        }
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let party_full = doc.save.party().len() >= offsets::PARTY_CAPACITY;
        if ui
            .add_enabled(!party_full, egui::Button::new("Take ➡ party"))
            .on_hover_text("Move the mon back to the party (stats recalculated, as in-game)")
            .clicked()
        {
            let party_record = box_to_party(&record);
            if doc
                .save
                .party_mut()
                .add(&party_record, &ot_name, &nickname)
                .is_ok()
            {
                let _ = doc.save.set_daycare(None);
                touched = true;
            }
        }
        if ui
            .button("🗑 Clear daycare")
            .on_hover_text("Mark the daycare empty (the mon is lost, as when picked up in-game)")
            .clicked()
        {
            let _ = doc.save.set_daycare(None);
            touched = true;
        }
    });

    touched
}

/// What the Gen 1 formula yields for this record at `level`.
fn expected_stats(snap: &MonSnapshot, level: u8) -> [u16; 5] {
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
