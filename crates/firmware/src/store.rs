//! The last good screen document, kept in flash so a failed fetch can
//! repaint it with the error in the status bar (RTC memory, which survives
//! deep sleep, is far too small for a 16 KiB body).
//!
//! Lives in the `screen` data partition (`partitions.csv`):
//!
//! | Offset | Contents |
//! |---|---|
//! | 0 | header: magic `u32`, body length `u32`, FNV-1a hash `u64` (LE) |
//! | 4096 | body |
//!
//! The header has its own sector and is written after the body, so a power
//! cut mid-save leaves a header whose hash no longer matches the body, which
//! [`Store::load`] rejects. Saves only happen when the content changes.

use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions::{self, PARTITION_TABLE_MAX_LEN};
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;
use log::{info, warn};
use screen_spec::{MAX_JSON_BYTES, content_hash};

const LABEL: &str = "screen";
const MAGIC: u32 = 0x5343_5231; // "SCR1"
const HEADER_LEN: usize = 16;
const BODY_OFFSET: u32 = FlashStorage::SECTOR_SIZE;

/// Handle on the `screen` partition.
pub struct Store {
    flash: FlashStorage<'static>,
    offset: u32,
}

struct Header {
    len: usize,
    hash: u64,
}

impl Store {
    /// Finds the `screen` partition. `None` (logged) if the device was
    /// flashed without `partitions.csv`.
    pub fn open(flash: FLASH<'static>) -> Option<Self> {
        let mut flash = FlashStorage::new(flash);
        let mut table_buf = [0u8; PARTITION_TABLE_MAX_LEN];
        let table = match partitions::read_partition_table(&mut flash, &mut table_buf) {
            Ok(t) => t,
            Err(e) => {
                warn!("store: partition table: {e:?}");
                return None;
            }
        };
        // Matched by label, not by `find_partition`, which needs the exact
        // sub-type. Type 1 = data.
        let entry = (0..table.len())
            .filter_map(|i| table.get_partition(i).ok())
            .find(|p| p.raw_type() == 1 && p.label_as_str() == LABEL);
        let Some(entry) = entry else {
            warn!("store: no `{LABEL}` partition (flash with partitions.csv)");
            return None;
        };
        if (entry.len() as usize) < BODY_OFFSET as usize + MAX_JSON_BYTES {
            warn!(
                "store: `{LABEL}` partition is too small ({} bytes)",
                entry.len()
            );
            return None;
        }
        Some(Store {
            flash,
            offset: entry.offset(),
        })
    }

    fn header(&mut self) -> Option<Header> {
        let mut raw = [0u8; HEADER_LEN];
        self.flash.read(self.offset, &mut raw).ok()?;
        let word = |i: usize| u32::from_le_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
        if word(0) != MAGIC {
            return None;
        }
        let len = word(4) as usize;
        if len > MAX_JSON_BYTES {
            return None;
        }
        let mut hash = [0u8; 8];
        hash.copy_from_slice(&raw[8..16]);
        Some(Header {
            len,
            hash: u64::from_le_bytes(hash),
        })
    }

    /// Copies the saved body into the start of `buf` and returns its length
    /// and hash, or `None` if nothing valid is saved.
    pub fn load(&mut self, buf: &mut [u8]) -> Option<(usize, u64)> {
        let header = self.header()?;
        let body = buf.get_mut(..header.len)?;
        self.flash.read(self.offset + BODY_OFFSET, body).ok()?;
        if content_hash(body) != header.hash {
            warn!("store: saved screen is corrupt");
            return None;
        }
        info!("store: loaded {} byte screen", header.len);
        Some((header.len, header.hash))
    }

    /// Saves `body` (whose hash is `hash`) unless it is already saved.
    pub fn save(&mut self, body: &[u8], hash: u64) {
        if self.header().is_some_and(|h| h.hash == hash) {
            return;
        }
        let mut header = [0u8; HEADER_LEN];
        header[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        header[4..8].copy_from_slice(&(body.len() as u32).to_le_bytes());
        header[8..16].copy_from_slice(&hash.to_le_bytes());
        let result = self
            .flash
            .write(self.offset + BODY_OFFSET, body)
            .and_then(|()| self.flash.write(self.offset, &header));
        match result {
            Ok(()) => info!("store: saved {} byte screen", body.len()),
            Err(e) => warn!("store: save failed: {e:?}"),
        }
    }
}
