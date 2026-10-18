//! App-owned block definitions and the session's version-bound registry.

use std::rc::Rc;

use azalea_block::BlockState;
use azalea_core::position::BlockPos;

use super::{
    BlockData, BlockTags, VersionBlockData, load_behaviors, registry_names, resolve_block_tags,
};
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

pub(crate) struct GlobalBlockDefinitions {
    definitions: Vec<(&'static str, Box<dyn Block>)>,
}

impl GlobalBlockDefinitions {
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
    fn register(&mut self, name: &'static str, definition: Box<dyn Block>) {
        self.definitions.push((name, definition));
    }
}

/// Session-owned metadata selection and numeric bindings to shared definitions.
pub(crate) struct BlockRegistry {
    definitions: Rc<GlobalBlockDefinitions>,
    protocol: i32,
    version_data: VersionBlockData,
    bindings: BlockBindings,
    tags: BlockTags,
}

impl BlockRegistry {
    pub(crate) fn new(definitions: Rc<GlobalBlockDefinitions>, protocol: i32) -> Self {
        let version_data = VersionBlockData::for_protocol(protocol);
        let bindings = BlockBindings::new(&definitions, version_data);
        Self {
            definitions,
            protocol,
            version_data,
            bindings,
            tags: BlockTags::default(),
        }
    }

    /// Keep wire identity even when two protocols share a canonical table.
    pub(crate) fn select_protocol(&mut self, protocol: i32) {
        if self.protocol != protocol {
            let version_data = VersionBlockData::for_protocol(protocol);
            if self.version_data.table_slot() != version_data.table_slot() {
                self.bindings = BlockBindings::new(&self.definitions, version_data);
            }
            self.version_data = version_data;
            self.protocol = protocol;
        }
        self.clear_tags();
    }

    pub(crate) fn replace_tags(&mut self, raw_tags: Vec<(String, Vec<i32>)>) {
        let names: Vec<_> = registry_names(self.version_data.table()).collect();
        self.tags = resolve_block_tags(raw_tags, &names);
    }

    pub(crate) fn clear_tags(&mut self) {
        self.tags = BlockTags::default();
    }

    pub(crate) fn tags(&self) -> &BlockTags {
        &self.tags
    }

    pub(crate) fn get(&self, state: BlockState) -> BlockStateView<'_> {
        BlockStateView {
            data: self.version_data.block_data(state),
            definition: self.bindings.get(&self.definitions, state),
        }
    }

    /// Select and invoke together so callbacks receive the queried state.
    pub(crate) fn animate_tick(
        &self,
        pos: BlockPos,
        level: &mut LevelContext<'_>,
        random: &mut fastrand::Rng,
    ) {
        let state = level.get_block_state(pos);
        self.get(state)
            .definition()
            .animate_tick(state, pos, level, random);
    }
}

/// Borrows metadata and a shared rule without copying either into the session.
pub(crate) struct BlockStateView<'a> {
    data: &'a BlockData,
    definition: &'a dyn Block,
}

impl BlockStateView<'_> {
    /// Includes the state's key/value `PropMap`, not definition properties.
    #[expect(dead_code, reason = "state metadata consumers migrate in a later step")]
    pub(crate) fn state_data(&self) -> &BlockData {
        self.data
    }

    /// The shared rule exposes immutable `BlockProperties` via `properties()`.
    pub(crate) fn definition(&self) -> &dyn Block {
        self.definition
    }
}

/// Private O(1) wire-state indices into the immutable global definitions.
struct BlockBindings {
    state_definitions: Vec<usize>,
}

impl BlockBindings {
    fn new(definitions: &GlobalBlockDefinitions, version_data: VersionBlockData) -> Self {
        let table = version_data.table();
        let mut state_definitions = Vec::with_capacity(table.len());
        for states in table.chunk_by(|a, b| a.id == b.id) {
            let index = definitions
                .definitions
                .iter()
                .position(|(name, _)| *name == states[0].id)
                .unwrap_or(0);
            state_definitions.extend(std::iter::repeat_n(index, states.len()));
        }
        Self { state_definitions }
    }

    /// O(1) wire-state lookup, including an out-of-table fallback.
    fn get<'a>(&self, definitions: &'a GlobalBlockDefinitions, state: BlockState) -> &'a dyn Block {
        let index = self
            .state_definitions
            .get(u32::from(state) as usize)
            .copied()
            .unwrap_or(0);
        definitions.definitions[index].1.as_ref()
    }
}
