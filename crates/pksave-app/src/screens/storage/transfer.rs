//! Pure party ⇄ box ⇄ daycare transfer semantics for the storage
//! screen: [`validate_drop`] classifies a drag between two slots into a
//! [`DropAction`] (or a user-facing refusal), and [`perform_drop`]
//! applies it to the save. Kept free of egui so the truth table is
//! unit-testable.

use pksave::gen1::offsets;
use pksave::gen1::pokemon::{box_to_party, party_to_box, MonView};
use pksave::gen1::save::SaveFile;

use super::slots::SlotId;

/// Where a dragged mon was released.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropTarget {
    /// A specific slot cell (occupied or one of the trailing empties).
    Slot(SlotId),
    /// A box tab: append into that box.
    BoxTab(usize),
    /// The party strip background / "to party" actions: append.
    Party,
}

/// A validated transfer, ready to apply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropAction {
    /// Nothing to do (dropped on itself or an equivalent no-op).
    NoOp,
    /// Reorder within the party.
    PartyReorder(usize, usize),
    /// Reorder within box `.0`.
    BoxReorder(usize, usize, usize),
    /// Positional swap between a party slot and an occupied box slot.
    SwapPartyBox {
        party: usize,
        box_n: usize,
        box_i: usize,
    },
    /// Append party slot `party` into box `box_n` (the game's deposit).
    Deposit { party: usize, box_n: usize },
    /// Append box slot into the party (the game's withdrawal).
    Withdraw { box_n: usize, box_i: usize },
    /// Move a box mon into another box, verbatim.
    MoveBoxToBox {
        from_box: usize,
        from_i: usize,
        to_box: usize,
    },
    /// Move party slot `.0` into the empty daycare.
    DepositDaycare(usize),
    /// Return the daycare mon to the party.
    TakeDaycare,
}

/// Why a drop is refused. `message()` is shown to the user.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DropError {
    PartyFull,
    BoxFull(usize),
    DaycareOccupied,
    OnlyPartyToDaycare,
    DaycareToPartyOnly,
}

impl DropError {
    pub fn message(&self) -> String {
        match self {
            DropError::PartyFull => format!("Party is full ({0}/{0})", offsets::PARTY_CAPACITY),
            DropError::BoxFull(n) => {
                format!("Box {} is full ({1}/{1})", n + 1, offsets::MONS_PER_BOX)
            }
            DropError::DaycareOccupied => "The daycare is already occupied".to_owned(),
            DropError::OnlyPartyToDaycare => {
                "Only a party Pokémon can enter the daycare".to_owned()
            }
            DropError::DaycareToPartyOnly => {
                "The daycare Pokémon can only return to the party".to_owned()
            }
        }
    }
}

/// Classify dragging `from` onto `to`. Occupied-on-occupied between
/// party and box swaps in place; anything onto an empty cell, a tab or
/// an area appends (deposit / withdraw / box move), exactly like the
/// game's PC plus a PKHeX-style swap.
pub fn validate_drop(
    save: &SaveFile,
    from: SlotId,
    to: DropTarget,
) -> Result<DropAction, DropError> {
    if DropTarget::Slot(from) == to {
        return Ok(DropAction::NoOp);
    }
    let party_len = save.party().len();
    let party_full = party_len >= offsets::PARTY_CAPACITY;
    let box_full = |n: usize| save.box_(n).len() >= offsets::MONS_PER_BOX;

    match (from, to) {
        // ---- from a party slot ----
        (SlotId::Party(i), DropTarget::Slot(SlotId::Party(j))) => {
            if i == j || j >= party_len {
                Ok(DropAction::NoOp) // party is contiguous: no empty targets
            } else {
                Ok(DropAction::PartyReorder(i, j))
            }
        }
        (SlotId::Party(i), DropTarget::Slot(SlotId::Box { box_n, index })) => {
            if index < save.box_(box_n).len() {
                Ok(DropAction::SwapPartyBox {
                    party: i,
                    box_n,
                    box_i: index,
                })
            } else if box_full(box_n) {
                Err(DropError::BoxFull(box_n))
            } else {
                Ok(DropAction::Deposit { party: i, box_n })
            }
        }
        (SlotId::Party(i), DropTarget::BoxTab(box_n)) => {
            if box_full(box_n) {
                Err(DropError::BoxFull(box_n))
            } else {
                Ok(DropAction::Deposit { party: i, box_n })
            }
        }
        (SlotId::Party(i), DropTarget::Slot(SlotId::Daycare)) => {
            if save.daycare().is_some() {
                Err(DropError::DaycareOccupied)
            } else {
                Ok(DropAction::DepositDaycare(i))
            }
        }
        (SlotId::Party(_), DropTarget::Party) => Ok(DropAction::NoOp),

        // ---- from a box slot ----
        (SlotId::Box { box_n, index }, DropTarget::Slot(SlotId::Party(j))) => {
            if j < party_len {
                Ok(DropAction::SwapPartyBox {
                    party: j,
                    box_n,
                    box_i: index,
                })
            } else if party_full {
                Err(DropError::PartyFull)
            } else {
                Ok(DropAction::Withdraw {
                    box_n,
                    box_i: index,
                })
            }
        }
        (SlotId::Box { box_n, index }, DropTarget::Party) => {
            if party_full {
                Err(DropError::PartyFull)
            } else {
                Ok(DropAction::Withdraw {
                    box_n,
                    box_i: index,
                })
            }
        }
        (
            SlotId::Box { box_n, index },
            DropTarget::Slot(SlotId::Box {
                box_n: to_box,
                index: to_i,
            }),
        ) => {
            if box_n == to_box {
                if to_i < save.box_(box_n).len() && to_i != index {
                    Ok(DropAction::BoxReorder(box_n, index, to_i))
                } else {
                    Ok(DropAction::NoOp)
                }
            } else if box_full(to_box) {
                Err(DropError::BoxFull(to_box))
            } else {
                Ok(DropAction::MoveBoxToBox {
                    from_box: box_n,
                    from_i: index,
                    to_box,
                })
            }
        }
        (SlotId::Box { box_n, index }, DropTarget::BoxTab(to_box)) => {
            if box_n == to_box {
                Ok(DropAction::NoOp)
            } else if box_full(to_box) {
                Err(DropError::BoxFull(to_box))
            } else {
                Ok(DropAction::MoveBoxToBox {
                    from_box: box_n,
                    from_i: index,
                    to_box,
                })
            }
        }
        (SlotId::Box { .. }, DropTarget::Slot(SlotId::Daycare)) => {
            Err(DropError::OnlyPartyToDaycare)
        }

        // ---- from the daycare ----
        (SlotId::Daycare, DropTarget::Slot(SlotId::Party(_)) | DropTarget::Party) => {
            if party_full {
                Err(DropError::PartyFull)
            } else {
                Ok(DropAction::TakeDaycare)
            }
        }
        (SlotId::Daycare, DropTarget::Slot(SlotId::Daycare)) => Ok(DropAction::NoOp),
        (SlotId::Daycare, _) => Err(DropError::DaycareToPartyOnly),
    }
}

/// Apply a validated action. Returns where the dragged mon ended up (so
/// selection can follow it), or a user-facing message if the save
/// refused the edit (stale indexes, undecodable daycare names, …).
pub fn perform_drop(save: &mut SaveFile, action: DropAction) -> Result<Option<SlotId>, String> {
    let bad = |e: &dyn std::fmt::Display| format!("Could not move the Pokémon: {e}");
    match action {
        DropAction::NoOp => Ok(None),
        DropAction::PartyReorder(i, j) => {
            save.party_mut().swap(i, j);
            Ok(Some(SlotId::Party(j)))
        }
        DropAction::BoxReorder(n, i, j) => {
            save.box_mut(n).swap(i, j);
            Ok(Some(SlotId::Box { box_n: n, index: j }))
        }
        DropAction::SwapPartyBox {
            party,
            box_n,
            box_i,
        } => save
            .swap_party_box(party, box_n, box_i)
            .map(|()| Some(SlotId::Party(party)))
            .map_err(|e| bad(&e)),
        DropAction::Deposit { party, box_n } => save
            .deposit(party, box_n)
            .map(|()| {
                Some(SlotId::Box {
                    box_n,
                    index: save.box_(box_n).len().saturating_sub(1),
                })
            })
            .map_err(|e| bad(&e)),
        DropAction::Withdraw { box_n, box_i } => save
            .withdraw(box_n, box_i)
            .map(|()| Some(SlotId::Party(save.party().len().saturating_sub(1))))
            .map_err(|e| bad(&e)),
        DropAction::MoveBoxToBox {
            from_box,
            from_i,
            to_box,
        } => save
            .move_box_to_box(from_box, from_i, to_box)
            .map(|()| {
                Some(SlotId::Box {
                    box_n: to_box,
                    index: save.box_(to_box).len().saturating_sub(1),
                })
            })
            .map_err(|e| bad(&e)),
        DropAction::DepositDaycare(p) => {
            if p >= save.party().len() {
                return Err("Could not move the Pokémon: it is no longer there".to_owned());
            }
            let (record, ot, nick) = {
                let party = save.party();
                let mon = party.mon(p);
                let mut rec = [0u8; offsets::PARTY_MON_SIZE];
                rec.copy_from_slice(mon.as_bytes());
                (rec, party.ot_name(p), party.nickname(p))
            };
            let box_record = party_to_box(&record);
            save.set_daycare(Some((&box_record, &ot, &nick)))
                .map_err(|e| bad(&e))?;
            save.party_mut().remove(p);
            Ok(Some(SlotId::Daycare))
        }
        DropAction::TakeDaycare => {
            let Some((record, nick, ot)) = save.daycare().map(|view| {
                let mut rec = [0u8; offsets::BOX_MON_SIZE];
                rec.copy_from_slice(view.mon().as_bytes());
                (rec, view.nickname(), view.ot_name())
            }) else {
                return Err("The daycare is empty".to_owned());
            };
            let party_record = box_to_party(&record);
            save.party_mut()
                .add(&party_record, &ot, &nick)
                .map_err(|e| bad(&e))?;
            let _ = save.set_daycare(None);
            Ok(Some(SlotId::Party(save.party().len().saturating_sub(1))))
        }
    }
}

/// The box numbers a [`DropAction`] writes to — the caller flushes the
/// live working copy to its bank slot for each (see the mutation
/// epilogue in `mod.rs`).
pub fn boxes_touched(action: DropAction) -> Vec<usize> {
    match action {
        DropAction::NoOp
        | DropAction::PartyReorder(..)
        | DropAction::DepositDaycare(_)
        | DropAction::TakeDaycare => vec![],
        DropAction::BoxReorder(n, ..)
        | DropAction::SwapPartyBox { box_n: n, .. }
        | DropAction::Deposit { box_n: n, .. }
        | DropAction::Withdraw { box_n: n, .. } => vec![n],
        DropAction::MoveBoxToBox {
            from_box, to_box, ..
        } => vec![from_box, to_box],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pksave::gen1::data::DEX_TO_INDEX;
    use pksave::gen1::pokemon::{BoxMonMut, MonMut, PartyMonMut};
    use pksave::gen1::save::GameVariant;

    fn party_mon(dex: usize, level: u8) -> [u8; offsets::PARTY_MON_SIZE] {
        let mut bytes = [0u8; offsets::PARTY_MON_SIZE];
        let mut mon = PartyMonMut::new(&mut bytes);
        mon.set_species(DEX_TO_INDEX[dex]);
        mon.set_level_coherent(level);
        bytes
    }

    fn box_mon(dex: usize, level: u8) -> [u8; offsets::BOX_MON_SIZE] {
        let mut bytes = [0u8; offsets::BOX_MON_SIZE];
        let mut mon = BoxMonMut::new(&mut bytes);
        mon.set_species(DEX_TO_INDEX[dex]);
        mon.set_level_coherent(level);
        bytes
    }

    /// party of 2, box 1 with 2 mons, box 2 full, daycare empty.
    fn fixture() -> SaveFile {
        let mut save = SaveFile::new_empty(GameVariant::RedBlue);
        save.party_mut()
            .add(&party_mon(25, 42), "ASH", "SPARKY")
            .unwrap();
        save.party_mut()
            .add(&party_mon(1, 10), "ASH", "BULBA")
            .unwrap();
        for _ in 0..2 {
            save.box_mut(1).add(&box_mon(4, 20), "RED", "CHAR").unwrap();
        }
        for _ in 0..offsets::MONS_PER_BOX {
            save.box_mut(2)
                .add(&box_mon(7, 9), "RED", "SQUIRT")
                .unwrap();
        }
        save
    }

    fn party(i: usize) -> SlotId {
        SlotId::Party(i)
    }
    fn boxed(box_n: usize, index: usize) -> SlotId {
        SlotId::Box { box_n, index }
    }

    #[test]
    fn party_to_party_is_reorder_or_noop() {
        let save = fixture();
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::Slot(party(1))),
            Ok(DropAction::PartyReorder(0, 1))
        );
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::Slot(party(0))),
            Ok(DropAction::NoOp)
        );
        // Trailing empty party cell: contiguous list, nothing to do.
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::Slot(party(4))),
            Ok(DropAction::NoOp)
        );
    }

    #[test]
    fn party_to_box_swaps_on_occupied_appends_on_empty() {
        let save = fixture();
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::Slot(boxed(1, 1))),
            Ok(DropAction::SwapPartyBox {
                party: 0,
                box_n: 1,
                box_i: 1
            })
        );
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::Slot(boxed(1, 7))),
            Ok(DropAction::Deposit { party: 0, box_n: 1 })
        );
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::BoxTab(1)),
            Ok(DropAction::Deposit { party: 0, box_n: 1 })
        );
    }

    #[test]
    fn full_box_refuses_appends_but_still_swaps() {
        let save = fixture();
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::BoxTab(2)),
            Err(DropError::BoxFull(2))
        );
        // A swap is capacity-neutral: allowed even into a full box.
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::Slot(boxed(2, 5))),
            Ok(DropAction::SwapPartyBox {
                party: 0,
                box_n: 2,
                box_i: 5
            })
        );
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::BoxTab(2)),
            Err(DropError::BoxFull(2))
        );
    }

    #[test]
    fn box_to_party_swaps_or_withdraws() {
        let save = fixture();
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Slot(party(1))),
            Ok(DropAction::SwapPartyBox {
                party: 1,
                box_n: 1,
                box_i: 0
            })
        );
        // Empty party cell: append (withdraw).
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Slot(party(3))),
            Ok(DropAction::Withdraw { box_n: 1, box_i: 0 })
        );
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Party),
            Ok(DropAction::Withdraw { box_n: 1, box_i: 0 })
        );
    }

    #[test]
    fn full_party_refuses_withdraw_but_still_swaps() {
        let mut save = fixture();
        for _ in 0..4 {
            save.party_mut()
                .add(&party_mon(1, 5), "ASH", "MON")
                .unwrap();
        }
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Party),
            Err(DropError::PartyFull)
        );
        assert_eq!(
            validate_drop(&save, SlotId::Daycare, DropTarget::Party),
            Err(DropError::PartyFull)
        );
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Slot(party(0))),
            Ok(DropAction::SwapPartyBox {
                party: 0,
                box_n: 1,
                box_i: 0
            })
        );
    }

    #[test]
    fn box_to_box_classification() {
        let save = fixture();
        // Same box: reorder on occupied, no-op on empty/self/tab.
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Slot(boxed(1, 1))),
            Ok(DropAction::BoxReorder(1, 0, 1))
        );
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Slot(boxed(1, 0))),
            Ok(DropAction::NoOp)
        );
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Slot(boxed(1, 9))),
            Ok(DropAction::NoOp)
        );
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::BoxTab(1)),
            Ok(DropAction::NoOp)
        );
        // Cross-box: append regardless of the exact cell.
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Slot(boxed(3, 0))),
            Ok(DropAction::MoveBoxToBox {
                from_box: 1,
                from_i: 0,
                to_box: 3
            })
        );
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::BoxTab(3)),
            Ok(DropAction::MoveBoxToBox {
                from_box: 1,
                from_i: 0,
                to_box: 3
            })
        );
    }

    #[test]
    fn daycare_rules() {
        let mut save = fixture();
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::Slot(SlotId::Daycare)),
            Ok(DropAction::DepositDaycare(0))
        );
        assert_eq!(
            validate_drop(&save, boxed(1, 0), DropTarget::Slot(SlotId::Daycare)),
            Err(DropError::OnlyPartyToDaycare)
        );

        perform_drop(&mut save, DropAction::DepositDaycare(0)).unwrap();
        assert_eq!(
            validate_drop(&save, party(0), DropTarget::Slot(SlotId::Daycare)),
            Err(DropError::DaycareOccupied)
        );
        assert_eq!(
            validate_drop(&save, SlotId::Daycare, DropTarget::Party),
            Ok(DropAction::TakeDaycare)
        );
        assert_eq!(
            validate_drop(&save, SlotId::Daycare, DropTarget::BoxTab(1)),
            Err(DropError::DaycareToPartyOnly)
        );
    }

    #[test]
    fn perform_drop_returns_the_new_location() {
        let mut save = fixture();
        assert_eq!(
            perform_drop(&mut save, DropAction::Deposit { party: 0, box_n: 1 }),
            Ok(Some(SlotId::Box { box_n: 1, index: 2 }))
        );
        assert_eq!(
            perform_drop(&mut save, DropAction::Withdraw { box_n: 1, box_i: 2 }),
            Ok(Some(SlotId::Party(1)))
        );
        assert_eq!(
            perform_drop(
                &mut save,
                DropAction::MoveBoxToBox {
                    from_box: 1,
                    from_i: 0,
                    to_box: 4
                }
            ),
            Ok(Some(SlotId::Box { box_n: 4, index: 0 }))
        );
    }

    #[test]
    fn daycare_round_trip_via_perform_drop() {
        let mut save = fixture();
        let nick = save.party().nickname(0);
        perform_drop(&mut save, DropAction::DepositDaycare(0)).unwrap();
        assert_eq!(save.party().len(), 1);
        assert_eq!(save.daycare().map(|d| d.nickname()), Some(nick.clone()));

        perform_drop(&mut save, DropAction::TakeDaycare).unwrap();
        assert!(save.daycare().is_none());
        assert_eq!(save.party().len(), 2);
        assert_eq!(save.party().nickname(1), nick);
    }
}
