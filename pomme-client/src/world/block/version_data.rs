//! Immutable metadata selected independently of the active session table.

use azalea_block::BlockState;

use super::{
    BEDROCK_LIGHT, BLOCK_DATA, BLOCK_TABLES, BlockData, DEFAULT_BEHAVIOR, NATIVE_SLOT, NO_FLUID,
    PropMap, SpecialCollision, block_data_slot, build_table,
};

/// A cheap, immutable handle to one cached protocol table. Aliases share data;
/// unsupported protocols use the native table, as the compatibility API does.
#[derive(Clone, Copy)]
pub(crate) struct VersionBlockData {
    slot: usize,
    table: &'static [BlockData],
}

impl VersionBlockData {
    /// Builds only the explicitly selected table, without reading or changing
    /// the active protocol. The cache owns the data for the process lifetime.
    pub(crate) fn for_protocol(protocol: i32) -> Self {
        let slot = block_data_slot(protocol).unwrap_or(NATIVE_SLOT);
        let table = BLOCK_TABLES[slot].get_or_init(|| build_table(&BLOCK_DATA[slot]));
        Self { slot, table }
    }

    /// Compatibility bridge; the caller supplies an already initialized slot.
    pub(super) fn from_initialized_slot(slot: usize) -> Self {
        Self {
            slot,
            table: BLOCK_TABLES[slot]
                .get()
                .expect("world::block::init must be called before use"),
        }
    }

    pub(super) fn table_slot(self) -> usize {
        self.slot
    }

    /// Unknown state ids retain the compatibility API's solid, unlit fallback.
    pub(crate) fn block_data(self, state: BlockState) -> &'static BlockData {
        static UNKNOWN: std::sync::LazyLock<BlockData> = std::sync::LazyLock::new(|| BlockData {
            id: "unknown",
            properties: PropMap::from_pairs(Vec::new()),
            behavior: DEFAULT_BEHAVIOR,
            shape: None,
            outline: None,
            is_air: false,
            collides: true,
            special_collision: SpecialCollision::None,
            blocks_motion: None,
            legacy_solid: None,
            replaceable: None,
            full_face_sturdy: None,
            large_collision_shape: false,
            collision_shape_uses_offset: false,
            outline_shape_uses_offset: false,
            position_offset: None,
            fluid: NO_FLUID,
            light: BEDROCK_LIGHT,
        });
        self.table
            .get(u32::from(state) as usize)
            .unwrap_or(&UNKNOWN)
    }
}
