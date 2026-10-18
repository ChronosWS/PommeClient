//! Runtime-bound block definitions with injected particle bindings.
//! Static protocol metadata remains independent of these
//! resource-bound objects.

mod torch;

use azalea_block::BlockState;

use super::{ACTIVE_TABLE, BLOCK_DATA, BLOCK_TABLES, Block};
use crate::particle::{ParticleStore, RegistrationError};

struct DefaultBlock;
impl Block for DefaultBlock {}

pub(crate) struct BlockDefinitions {
    definitions: Vec<(&'static str, Box<dyn Block>)>,
    state_definitions: Vec<usize>,
    table_slot: Option<usize>,
}

impl BlockDefinitions {
    pub(crate) fn new(particles: &ParticleStore) -> Result<Self, RegistrationError> {
        let mut registry = Self {
            definitions: vec![("", Box::new(DefaultBlock))],
            state_definitions: Vec::new(),
            table_slot: None,
        };
        torch::register_definitions(&mut registry, particles)?;
        registry.refresh_protocol();
        Ok(registry)
    }

    fn register(&mut self, name: &'static str, definition: Box<dyn Block>) {
        self.definitions.push((name, definition));
    }

    /// Check once per pass: connections can select their protocol after
    /// bootstrap.
    pub(crate) fn refresh_protocol(&mut self) {
        self.bind_table(ACTIVE_TABLE.load(std::sync::atomic::Ordering::Acquire));
    }

    fn bind_table(&mut self, slot: usize) {
        if self.table_slot == Some(slot) {
            return;
        }
        let table = BLOCK_TABLES[slot].get_or_init(|| super::build_table(&BLOCK_DATA[slot]));
        self.state_definitions.clear();
        for states in table.chunk_by(|a, b| a.id == b.id) {
            let index = self
                .definitions
                .iter()
                .position(|(name, _)| *name == states[0].id)
                .unwrap_or(0);
            self.state_definitions
                .extend(std::iter::repeat_n(index, states.len()));
        }
        self.table_slot = Some(slot);
    }

    /// O(1) wire-state dispatch; the returned rule borrows this collection.
    pub(crate) fn get(&self, state: BlockState) -> &dyn Block {
        let index = self
            .state_definitions
            .get(u32::from(state) as usize)
            .copied()
            .unwrap_or(0);
        self.definitions[index].1.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use azalea_block::BlockState;
    use azalea_core::position::BlockPos;
    use glam::DVec3;

    use super::*;
    use crate::world::block::{LevelContext, all_states, block_id, find_state};

    #[test]
    fn property_states_share_definitions_and_default_blocks_are_noop() {
        crate::world::block::init("26.2");
        let mut store = crate::particle::tests::empty_store();
        let definitions = BlockDefinitions::new(&store).unwrap();
        for id in [
            "torch",
            "wall_torch",
            "soul_torch",
            "soul_wall_torch",
            "copper_torch",
            "copper_wall_torch",
            "redstone_torch",
            "redstone_wall_torch",
        ] {
            let definition = definitions.get(find_state(id, &[]));
            for (state, _) in all_states().filter(|(_, data)| data.id == id) {
                assert!(std::ptr::eq(definition, definitions.get(state)));
            }
        }
        let pos = BlockPos::new(0, 0, 0);
        let mut level = LevelContext {
            particles: store.local_spawns(DVec3::ZERO),
        };
        let mut random = fastrand::Rng::with_seed(42);
        for id in ["air", "stone", "campfire", "soul_campfire", "water", "lava"] {
            let state = find_state(id, &[]);
            assert_eq!(block_id(state), id);
            assert!(std::ptr::eq(
                definitions.get(state),
                definitions.definitions[0].1.as_ref()
            ));
            definitions
                .get(state)
                .animate_tick(state, pos, &mut level, &mut random);
        }
        let unknown = BlockState::try_from(u32::from(u16::MAX)).unwrap();
        assert_eq!(block_id(unknown), "unknown");
        assert!(std::ptr::eq(
            definitions.get(unknown),
            definitions.definitions[0].1.as_ref()
        ));
        definitions
            .get(unknown)
            .animate_tick(unknown, pos, &mut level, &mut random);
        assert_eq!(random.get_seed(), 42, "no-op blocks drew randomness");
        let chunks = crate::world::chunk::ChunkStore::with_origin_chunk();
        store.tick(&chunks);
        assert!(store.extract(0.0, DVec3::ZERO, &chunks).is_empty());
    }

    #[test]
    fn protocol_mapping_uses_each_tables_wire_states_and_rebuilds() {
        let store = crate::particle::tests::empty_store();
        let mut definitions = BlockDefinitions::new(&store).unwrap();
        // Bind explicit tables rather than changing the process-wide active
        // protocol while other tests read native block properties.
        let mut torch_ids = Vec::new();
        for protocol in [763, 774, pomme_protocol::version::NATIVE.protocol, 763] {
            let slot = super::super::prewarm_protocol(protocol);
            definitions.bind_table(slot);
            let table = BLOCK_TABLES[slot].get().unwrap();
            assert_eq!(definitions.state_definitions.len(), table.len());
            torch_ids.push(table.iter().position(|data| data.id == "torch").unwrap());
            for (id, data) in table.iter().enumerate() {
                let expected = definitions
                    .definitions
                    .iter()
                    .position(|(name, _)| *name == data.id)
                    .unwrap_or(0);
                assert!(std::ptr::eq(
                    definitions.get(BlockState::try_from(id as u32).unwrap()),
                    definitions.definitions[expected].1.as_ref(),
                ));
            }
            for (floor, wall) in [
                ("torch", "wall_torch"),
                ("redstone_torch", "redstone_wall_torch"),
            ] {
                let state = |name| {
                    BlockState::try_from(table.iter().position(|d| d.id == name).unwrap() as u32)
                        .unwrap()
                };
                assert!(!std::ptr::eq(
                    definitions.get(state(floor)),
                    definitions.get(state(wall))
                ));
            }
        }
        assert!(torch_ids.windows(2).any(|pair| pair[0] != pair[1]));
        definitions.refresh_protocol();
        assert_eq!(
            definitions.table_slot,
            Some(ACTIVE_TABLE.load(std::sync::atomic::Ordering::Acquire))
        );
    }
}
