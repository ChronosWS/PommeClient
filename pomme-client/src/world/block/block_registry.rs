//! App-owned block rules and the level's current protocol bindings.

use azalea_block::BlockState;
use azalea_core::position::BlockPos;

use super::{BLOCK_DATA, BLOCK_TABLES, active_table_slot, load_behaviors};
use crate::world::level::LevelContext;

/// Shared, immutable block rules, independent of the compact wire state ID.
pub trait Block {
    #[expect(
        dead_code,
        reason = "definition properties precede mining consumer migration"
    )]
    fn properties(&self) -> &BlockProperties;

    fn animate_tick(
        &self,
        _state: BlockState,
        _pos: BlockPos,
        _level: &mut LevelContext<'_>,
        _random: &mut fastrand::Rng,
    ) {
    }
}

/// Immutable definition data shared by all states of one registered block.
/// Distinct from the state's key/value properties in [`super::PropMap`].
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockProperties {
    pub destroy_time: f32,
    pub requires_correct_tool_for_drops: bool,
}

struct DefaultBlock {
    #[expect(
        dead_code,
        reason = "definition properties precede mining consumer migration"
    )]
    properties: BlockProperties,
}

impl Block for DefaultBlock {
    fn properties(&self) -> &BlockProperties {
        &self.properties
    }
}

/// Selects and constructs block implementations, without owning definitions
/// or protocol bindings. Add name-specific selections here as rules land;
/// currently every name uses the default no-op animation behavior.
pub(crate) struct BlockBehaviorCatalog;

impl BlockBehaviorCatalog {
    pub(crate) fn create(&self, _name: &str, properties: BlockProperties) -> Box<dyn Block> {
        Box::new(DefaultBlock { properties })
    }
}

pub(crate) struct BlockRegistry {
    definitions: Vec<(&'static str, Box<dyn Block>)>,
}

impl BlockRegistry {
    pub(crate) fn new() -> Self {
        let catalog = BlockBehaviorCatalog;
        let mut entries: Vec<_> = load_behaviors().iter().collect();
        entries.sort_unstable_by_key(|(name, _)| *name);
        let mut registry = Self {
            definitions: Vec::with_capacity(entries.len() + 1),
        };
        registry.register("", catalog.create("", BlockProperties::default()));
        for (name, entry) in entries {
            registry.register(name.as_str(), catalog.create(name, entry.properties()));
        }
        registry
    }

    /// Acquire a definition by its unnamespaced data name, independent of the
    /// active protocol. Unknown names use the same fallback as unknown states.
    #[expect(
        dead_code,
        reason = "named acquisition precedes mining consumer migration"
    )]
    pub(crate) fn get(&self, name: &str) -> &dyn Block {
        self.definitions
            .iter()
            .find(|(registered, _)| *registered == name)
            .unwrap_or(&self.definitions[0])
            .1
            .as_ref()
    }

    /// Register one shared rule for all property states of a known block.
    pub(crate) fn register(&mut self, name: &'static str, definition: Box<dyn Block>) {
        self.definitions.push((name, definition));
    }
}

/// Numeric bindings remain level-owned; rules are borrowed from the app.
pub(crate) struct BlockBindings {
    state_definitions: Vec<usize>,
    table_slot: Option<usize>,
    definition_count: usize,
}

impl BlockBindings {
    pub(crate) fn new() -> Self {
        Self {
            state_definitions: Vec::new(),
            table_slot: None,
            definition_count: 0,
        }
    }

    /// Connections can select their protocol after bootstrap; check each pass.
    pub(crate) fn refresh_protocol(&mut self, registry: &BlockRegistry) {
        self.bind_table(registry, active_table_slot());
    }

    fn bind_table(&mut self, registry: &BlockRegistry, slot: usize) {
        // Registration only appends, so the count invalidates every level's
        // mapping without sharing mutable binding state.
        if self.table_slot == Some(slot) && self.definition_count == registry.definitions.len() {
            return;
        }
        let table = BLOCK_TABLES[slot].get_or_init(|| super::build_table(&BLOCK_DATA[slot]));
        self.state_definitions.clear();
        for states in table.chunk_by(|a, b| a.id == b.id) {
            let index = registry
                .definitions
                .iter()
                .position(|(name, _)| *name == states[0].id)
                .unwrap_or(0);
            self.state_definitions
                .extend(std::iter::repeat_n(index, states.len()));
        }
        self.table_slot = Some(slot);
        self.definition_count = registry.definitions.len();
    }

    /// O(1) wire-state lookup, including an out-of-table fallback.
    pub(crate) fn get<'a>(&self, registry: &'a BlockRegistry, state: BlockState) -> &'a dyn Block {
        let index = self
            .state_definitions
            .get(u32::from(state) as usize)
            .copied()
            .unwrap_or(0);
        registry.definitions[index].1.as_ref()
    }

    /// Select and invoke together so callbacks always receive the queried
    /// state.
    pub(crate) fn animate_tick(
        &self,
        registry: &BlockRegistry,
        pos: BlockPos,
        level: &mut LevelContext<'_>,
        random: &mut fastrand::Rng,
    ) {
        let state = level.get_block_state(pos);
        self.get(registry, state)
            .animate_tick(state, pos, level, random);
    }
}
