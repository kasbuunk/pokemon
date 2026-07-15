//! The daycare: one optional box-format mon plus its nickname and OT.
//!
//! Layout (FORMAT.md, main block): `wDayCareInUse` (0x2CF4, 0/1), the
//! nickname (0x2CF5, 11 bytes), the OT name (0x2D00, 11 bytes) and the
//! 33-byte box-format mon record (0x2D0B). When the in-use byte is 0 the
//! game ignores the other fields entirely, so clearing the daycare only
//! writes that one byte and leaves the stale mon bytes in place — that is
//! exactly what the game does when a mon is picked up.

use super::offsets;
use super::pokemon::{BoxMonMut, BoxMonView};
use super::save::SaveFile;
use super::text::{self, TextError};

/// Read-only view of an occupied daycare.
#[derive(Debug, Clone, Copy)]
pub struct DaycareView<'a> {
    buf: &'a [u8],
}

/// Mutable view of an occupied daycare.
#[derive(Debug)]
pub struct DaycareMut<'a> {
    buf: &'a mut [u8],
}

impl SaveFile {
    /// The daycare occupant, or `None` when `wDayCareInUse` is 0.
    pub fn daycare(&self) -> Option<DaycareView<'_>> {
        (self.buf()[offsets::DAYCARE_IN_USE] != 0).then(|| DaycareView { buf: self.buf() })
    }

    /// Mutable access to the daycare occupant, or `None` when empty.
    /// Marks the file edited.
    pub fn daycare_mut(&mut self) -> Option<DaycareMut<'_>> {
        if self.buf()[offsets::DAYCARE_IN_USE] == 0 {
            return None;
        }
        Some(DaycareMut {
            buf: self.buf_mut(),
        })
    }

    /// Deposit into or clear the daycare.
    ///
    /// - `Some((mon, ot_name, nickname))` sets the in-use byte to 1 and
    ///   writes all three fields. Names are validated *before* anything
    ///   is written.
    /// - `None` clears the in-use byte only; the stale mon/name bytes
    ///   stay in place (they are dead data to the game).
    pub fn set_daycare(
        &mut self,
        occupant: Option<(&[u8; offsets::BOX_MON_SIZE], &str, &str)>,
    ) -> Result<(), TextError> {
        match occupant {
            None => {
                self.buf_mut()[offsets::DAYCARE_IN_USE] = 0;
            }
            Some((mon, ot_name, nickname)) => {
                let ot = text::encode(ot_name, offsets::NAME_LEN)?;
                let nick = text::encode(nickname, offsets::NAME_LEN)?;
                let buf = self.buf_mut();
                buf[offsets::DAYCARE_IN_USE] = 1;
                buf[offsets::DAYCARE_NICKNAME..offsets::DAYCARE_NICKNAME + offsets::NAME_LEN]
                    .copy_from_slice(&nick);
                buf[offsets::DAYCARE_OT..offsets::DAYCARE_OT + offsets::NAME_LEN]
                    .copy_from_slice(&ot);
                buf[offsets::DAYCARE_MON..offsets::DAYCARE_MON + offsets::BOX_MON_SIZE]
                    .copy_from_slice(mon);
            }
        }
        Ok(())
    }
}

impl<'a> DaycareView<'a> {
    /// The deposited mon (box format).
    pub fn mon(&self) -> BoxMonView<'a> {
        BoxMonView::new(
            &self.buf[offsets::DAYCARE_MON..offsets::DAYCARE_MON + offsets::BOX_MON_SIZE],
        )
    }

    /// Decoded OT name.
    pub fn ot_name(&self) -> String {
        text::decode(&self.buf[offsets::DAYCARE_OT..offsets::DAYCARE_OT + offsets::NAME_LEN])
    }

    /// Decoded nickname.
    pub fn nickname(&self) -> String {
        text::decode(
            &self.buf[offsets::DAYCARE_NICKNAME..offsets::DAYCARE_NICKNAME + offsets::NAME_LEN],
        )
    }
}

impl DaycareMut<'_> {
    /// Read-only view of the same occupant.
    pub fn as_view(&self) -> DaycareView<'_> {
        DaycareView { buf: self.buf }
    }

    /// The deposited mon (box format).
    pub fn mon(&self) -> BoxMonView<'_> {
        self.as_view().mon()
    }

    /// Mutable view of the deposited mon.
    pub fn mon_mut(&mut self) -> BoxMonMut<'_> {
        BoxMonMut::new(
            &mut self.buf[offsets::DAYCARE_MON..offsets::DAYCARE_MON + offsets::BOX_MON_SIZE],
        )
    }

    /// Decoded OT name.
    pub fn ot_name(&self) -> String {
        self.as_view().ot_name()
    }

    /// Decoded nickname.
    pub fn nickname(&self) -> String {
        self.as_view().nickname()
    }

    /// Encode and store the OT name. The buffer is untouched on error.
    pub fn set_ot_name(&mut self, name: &str) -> Result<(), TextError> {
        let encoded = text::encode(name, offsets::NAME_LEN)?;
        self.buf[offsets::DAYCARE_OT..offsets::DAYCARE_OT + offsets::NAME_LEN]
            .copy_from_slice(&encoded);
        Ok(())
    }

    /// Encode and store the nickname. The buffer is untouched on error.
    pub fn set_nickname(&mut self, name: &str) -> Result<(), TextError> {
        let encoded = text::encode(name, offsets::NAME_LEN)?;
        self.buf[offsets::DAYCARE_NICKNAME..offsets::DAYCARE_NICKNAME + offsets::NAME_LEN]
            .copy_from_slice(&encoded);
        Ok(())
    }
}
