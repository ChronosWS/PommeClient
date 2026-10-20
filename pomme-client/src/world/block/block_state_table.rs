//! Immutable, version-selected state-ID table, independent of the active
//! session. Each entry is one state's `BlockStateData`; this handle selects the
//! whole table.

use std::collections::HashMap;

use azalea_block::BlockState;

use super::{
    BEDROCK_LIGHT, BLOCK_DATA, BLOCK_TABLES, BlockStateData, NATIVE_SLOT, NO_FLUID,
    PLACEABLE_BLOCKS, PropMap, SpecialCollision, block_data_slot, build_table,
};

/// A cheap, immutable handle to one cached version-selected state-ID table.
/// Aliases share data; unsupported protocols use the native table.
#[derive(Clone, Copy)]
pub(crate) struct BlockStateTable {
    slot: usize,
    table: &'static [BlockStateData],
}

impl BlockStateTable {
    /// Builds only the explicitly selected table, without reading or changing
    /// the active protocol. The cache owns the data for the process lifetime.
    pub(crate) fn for_protocol(protocol: i32) -> Self {
        let slot = block_data_slot(protocol).unwrap_or(NATIVE_SLOT);
        let table = BLOCK_TABLES[slot].get_or_init(|| build_table(&BLOCK_DATA[slot]));
        Self { slot, table }
    }

    pub(crate) fn table_slot(self) -> usize {
        self.slot
    }

    /// Validates a wire-side state id in this selected table.
    pub(crate) fn try_state(self, id: u32) -> Option<BlockState> {
        if (id as usize) < self.table.len() {
            BlockState::try_from(id).ok()
        } else {
            None
        }
    }

    pub(crate) fn registry_name(self, registry_id: u32) -> Option<&'static str> {
        super::registry_names(self.table).nth(registry_id as usize)
    }

    pub(crate) fn first_state_of(self, name: &str) -> Option<BlockState> {
        self.table
            .iter()
            .position(|block_state_data| block_state_data.id() == name)
            .map(|id| BlockState::try_from(id as u32).expect("table indices fit the id repr"))
    }

    #[cfg(any(test, feature = "cpu-bench"))]
    pub(crate) fn find_state(self, name: &str, props: &[(&str, &str)]) -> BlockState {
        let id = self
            .table
            .iter()
            .position(|block_state_data| {
                block_state_data.id() == name
                    && props
                        .iter()
                        .all(|&(k, v)| block_state_data.properties().get(k) == Some(v))
            })
            .unwrap_or_else(|| panic!("no state {name} {props:?}"));
        BlockState::try_from(id as u32).expect("table indices fit the id repr")
    }

    /// State metadata in wire-id order for binding the selected definitions.
    pub(super) fn table(self) -> &'static [BlockStateData] {
        self.table
    }

    /// Only single-state blocks support placement prediction by item name.
    /// Names are unprefixed, matching the item registry.
    pub(crate) fn placeable_block_for_item(self, item_name: &str) -> Option<BlockState> {
        let map = PLACEABLE_BLOCKS[self.slot].get_or_init(|| {
            let mut seen: HashMap<&'static str, Option<BlockState>> = HashMap::new();
            for (id, data) in self.table.iter().enumerate() {
                let state = BlockState::try_from(id as u32).expect("table indices fit the id repr");
                seen.entry(data.id)
                    .and_modify(|v| *v = None)
                    .or_insert(Some(state));
            }
            seen.into_iter()
                .filter_map(|(name, state)| state.map(|s| (name, s)))
                .collect()
        });
        map.get(item_name).copied()
    }

    /// Unknown state ids use a solid, unlit fallback.
    pub(crate) fn block_state_data(self, state: BlockState) -> &'static BlockStateData {
        static UNKNOWN: std::sync::LazyLock<BlockStateData> =
            std::sync::LazyLock::new(|| BlockStateData {
                id: "unknown",
                properties: PropMap::from_pairs(Vec::new()),
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
