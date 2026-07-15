//! Terminated item lists: the bag (20 slots) and the item PC (50 slots).
//!
//! Layout (`wBagItems` / `wBoxItems`): a count byte, then `count` pairs
//! of `[item id, quantity]`, then a `0xFF` terminator right after the
//! last pair. The field reserves `2 * capacity + 1` bytes so a full list
//! still fits its terminator.
//!
//! Structural edits ([`ItemListMut::add`], [`ItemListMut::remove`], …)
//! touch **only** the list region (count byte + items field). `remove`
//! shifts the following pairs down and writes the terminator at the new
//! end; bytes past that terminator are left as-is (stale pair bytes are
//! harmless — the game never reads past the terminator).
//!
//! Reading is fault-tolerant: a corrupt count byte larger than the
//! capacity reads as `capacity` (so iteration stays in bounds) and is
//! reported by [`diagnose_item_list`] instead of failing.

use thiserror::Error;

use super::data::ITEM_NAMES;
use super::offsets;
use super::save::SaveFile;
use crate::{Diagnostic, Severity};

/// Terminator byte after the last `[id, qty]` pair.
pub const LIST_TERMINATOR: u8 = 0xFF;
/// Largest quantity one slot holds; setters clamp to `1..=MAX_QTY`.
pub const MAX_QTY: u8 = 99;

/// Static description of one item list (offsets + capacity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemListSpec {
    /// File offset of the count byte.
    pub count_offset: usize,
    /// File offset of the first `[id, qty]` pair.
    pub items_offset: usize,
    /// Maximum number of entries.
    pub capacity: usize,
    /// Human-readable name for diagnostics.
    pub label: &'static str,
}

impl ItemListSpec {
    /// The whole byte region this list may touch: count byte plus the
    /// `2 * capacity + 1` bytes of pairs and terminator.
    pub fn region(&self) -> core::ops::Range<usize> {
        self.count_offset..self.items_offset + 2 * self.capacity + 1
    }
}

/// The bag (`wNumBagItems`/`wBagItems`, 20 slots).
pub const BAG_LIST: ItemListSpec = ItemListSpec {
    count_offset: offsets::BAG_ITEM_COUNT,
    items_offset: offsets::BAG_ITEMS,
    capacity: offsets::BAG_CAPACITY,
    label: "bag",
};

/// The item PC (`wNumBoxItems`/`wBoxItems`, 50 slots).
pub const PC_LIST: ItemListSpec = ItemListSpec {
    count_offset: offsets::PC_ITEM_COUNT,
    items_offset: offsets::PC_ITEMS,
    capacity: offsets::PC_ITEM_CAPACITY,
    label: "item PC",
};

/// Item list editing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ItemError {
    /// The list already holds `capacity` entries.
    #[error("item list is full ({capacity} slots)")]
    CapacityFull {
        /// Capacity of the list that was full (20 bag / 50 PC).
        capacity: usize,
    },
}

/// Read-only view of one item list.
#[derive(Debug, Clone, Copy)]
pub struct ItemListView<'a> {
    buf: &'a [u8],
    spec: ItemListSpec,
}

/// Mutable handle on one item list. Obtaining it marks the save edited.
#[derive(Debug)]
pub struct ItemListMut<'a> {
    buf: &'a mut [u8],
    spec: ItemListSpec,
}

impl<'a> ItemListView<'a> {
    /// Number of entries: the stored count byte, clamped to the
    /// capacity if corrupt (see [`diagnose_item_list`]).
    pub fn len(&self) -> usize {
        (self.buf[self.spec.count_offset] as usize).min(self.spec.capacity)
    }

    /// Whether the list has no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Entry `index` as `(item id, quantity)`, or `None` past the end.
    pub fn get(&self, index: usize) -> Option<(u8, u8)> {
        (index < self.len()).then(|| {
            let at = self.spec.items_offset + 2 * index;
            (self.buf[at], self.buf[at + 1])
        })
    }

    /// Iterator over `(item id, quantity)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (u8, u8)> + 'a {
        let spec = self.spec;
        let buf = self.buf;
        (0..(buf[spec.count_offset] as usize).min(spec.capacity)).map(move |i| {
            let at = spec.items_offset + 2 * i;
            (buf[at], buf[at + 1])
        })
    }
}

impl ItemListMut<'_> {
    fn view(&self) -> ItemListView<'_> {
        ItemListView {
            buf: self.buf,
            spec: self.spec,
        }
    }

    /// See [`ItemListView::len`].
    pub fn len(&self) -> usize {
        self.view().len()
    }

    /// Whether the list has no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// See [`ItemListView::get`].
    pub fn get(&self, index: usize) -> Option<(u8, u8)> {
        self.view().get(index)
    }

    fn pair_offset(&self, index: usize) -> usize {
        assert!(
            index < self.len(),
            "item index {index} out of range (len {})",
            self.len()
        );
        self.spec.items_offset + 2 * index
    }

    /// Set the quantity of entry `index`, clamped to `1..=99`.
    /// Panics if `index >= len()`.
    pub fn set_qty(&mut self, index: usize, qty: u8) {
        let at = self.pair_offset(index);
        self.buf[at + 1] = qty.clamp(1, MAX_QTY);
    }

    /// Set the item id of entry `index`. Panics if `index >= len()`.
    pub fn set_id(&mut self, index: usize, id: u8) {
        let at = self.pair_offset(index);
        self.buf[at] = id;
    }

    /// Append an entry (quantity clamped to `1..=99`) and re-write the
    /// terminator after it. Fails when the list is full, leaving the
    /// buffer untouched.
    pub fn add(&mut self, id: u8, qty: u8) -> Result<(), ItemError> {
        let len = self.len();
        if len == self.spec.capacity {
            return Err(ItemError::CapacityFull {
                capacity: self.spec.capacity,
            });
        }
        let at = self.spec.items_offset + 2 * len;
        self.buf[at] = id;
        self.buf[at + 1] = qty.clamp(1, MAX_QTY);
        self.buf[at + 2] = LIST_TERMINATOR;
        self.buf[self.spec.count_offset] = (len + 1) as u8;
        Ok(())
    }

    /// Remove entry `index`: shift the following pairs down one slot,
    /// decrement the count and write the terminator at the new end.
    /// Bytes past the terminator are left untouched. Panics if
    /// `index >= len()`.
    pub fn remove(&mut self, index: usize) {
        let len = self.len();
        assert!(index < len, "item index {index} out of range (len {len})");
        let base = self.spec.items_offset;
        for slot in index..len - 1 {
            let at = base + 2 * slot;
            self.buf[at] = self.buf[at + 2];
            self.buf[at + 1] = self.buf[at + 3];
        }
        self.buf[base + 2 * (len - 1)] = LIST_TERMINATOR;
        self.buf[self.spec.count_offset] = (len - 1) as u8;
    }

    /// Swap entries `a` and `b`. Panics if either is `>= len()`.
    pub fn swap(&mut self, a: usize, b: usize) {
        let at_a = self.pair_offset(a);
        let at_b = self.pair_offset(b);
        self.buf.swap(at_a, at_b);
        self.buf.swap(at_a + 1, at_b + 1);
    }
}

impl SaveFile {
    /// Read-only view of the bag.
    pub fn bag_items(&self) -> ItemListView<'_> {
        ItemListView {
            buf: self.buf(),
            spec: BAG_LIST,
        }
    }

    /// Mutable handle on the bag (marks the save edited).
    pub fn bag_items_mut(&mut self) -> ItemListMut<'_> {
        ItemListMut {
            buf: self.buf_mut(),
            spec: BAG_LIST,
        }
    }

    /// Read-only view of the item PC.
    pub fn pc_items(&self) -> ItemListView<'_> {
        ItemListView {
            buf: self.buf(),
            spec: PC_LIST,
        }
    }

    /// Mutable handle on the item PC (marks the save edited).
    pub fn pc_items_mut(&mut self) -> ItemListMut<'_> {
        ItemListMut {
            buf: self.buf_mut(),
            spec: PC_LIST,
        }
    }
}

/// Non-fatal findings about one item list:
///
/// - `W-ITEMS-COUNT`: count byte exceeds the capacity,
/// - `W-ITEMS-TERMINATOR`: the byte after the last entry is not `0xFF`,
/// - `W-ITEMS-UNKNOWN-ID`: an entry's id has no item name,
/// - `W-ITEMS-QTY-ZERO`: an entry's quantity is 0.
pub fn diagnose_item_list(buf: &[u8], spec: &ItemListSpec) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let warn = |code, message, at: usize| Diagnostic {
        severity: Severity::Warning,
        code,
        message,
        span: Some(at..at + 1),
    };

    let count = buf[spec.count_offset] as usize;
    if count > spec.capacity {
        diags.push(warn(
            "W-ITEMS-COUNT",
            format!(
                "{} count is {count} but the list holds at most {}",
                spec.label, spec.capacity
            ),
            spec.count_offset,
        ));
    }
    let len = count.min(spec.capacity);

    let terminator_at = spec.items_offset + 2 * len;
    if buf[terminator_at] != LIST_TERMINATOR {
        diags.push(warn(
            "W-ITEMS-TERMINATOR",
            format!(
                "{} terminator after entry {len} is 0x{:02X}, expected 0xFF",
                spec.label, buf[terminator_at]
            ),
            terminator_at,
        ));
    }

    for index in 0..len {
        let at = spec.items_offset + 2 * index;
        let (id, qty) = (buf[at], buf[at + 1]);
        if ITEM_NAMES[id as usize].is_empty() {
            diags.push(warn(
                "W-ITEMS-UNKNOWN-ID",
                format!(
                    "{} entry {index} has unknown item id 0x{id:02X}",
                    spec.label
                ),
                at,
            ));
        }
        if qty == 0 {
            diags.push(warn(
                "W-ITEMS-QTY-ZERO",
                format!("{} entry {index} has quantity 0", spec.label),
                at + 1,
            ));
        }
    }
    diags
}
