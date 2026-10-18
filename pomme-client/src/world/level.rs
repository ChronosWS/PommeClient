//! Conceptual equivalent to Vanilla ClientLevel/Level.  Provides access to
//! resources other game systems need, without exposing internals of GameState
//! generally.
use azalea_block::BlockState;
use azalea_core::position::BlockPos;
use glam::DVec3;

use super::block::block_registry::{BlockBindings, BlockRegistry};
use super::chunk::ChunkStore;

/// World services for a block callback; never retains a chunk snapshot.
pub struct LevelContext<'a> {
    chunks: &'a ChunkStore,
}

impl<'a> LevelContext<'a> {
    pub(crate) fn new(chunks: &'a ChunkStore) -> Self {
        Self { chunks }
    }

    /// Missing chunks and positions outside the dimension height read as air.
    pub fn get_block_state(&self, pos: BlockPos) -> BlockState {
        self.chunks.get_block_state(pos.x, pos.y, pos.z)
    }
}

/// Owns protocol bindings and persistent coordinate-sampling randomness only.
/// Block rules, chunks and other world runtimes remain externally owned.
pub struct LevelRuntime {
    // Preserve vanilla's distribution and draw order, not its seeded sequence.
    animation_rng: fastrand::Rng,
    bindings: BlockBindings,
}

impl LevelRuntime {
    pub(crate) fn new() -> Self {
        Self {
            animation_rng: fastrand::Rng::new(),
            bindings: BlockBindings::new(),
        }
    }

    /// Vanilla `ClientLevel.animateTick`, called once per simulation tick.
    pub(crate) fn animate_tick(
        &mut self,
        registry: &BlockRegistry,
        chunks: &ChunkStore,
        player_pos: DVec3,
    ) {
        self.bindings.refresh_protocol(registry);
        animate_tick_with_draw(
            &self.bindings,
            registry,
            player_pos,
            |radius| self.animation_rng.i32(0..radius),
            &mut LevelContext::new(chunks),
            fastrand::Rng::new(),
        );
    }
}

/// Own the effect stream for this pass; all sampled callbacks borrow it.
fn animate_tick_with_draw(
    bindings: &BlockBindings,
    registry: &BlockRegistry,
    player_pos: DVec3,
    next: impl FnMut(i32) -> i32,
    level: &mut LevelContext<'_>,
    mut random: fastrand::Rng,
) {
    sample_animation_blocks(player_pos, next, |pos| {
        bindings.animate_tick(registry, pos, level, &mut random);
    });
}

/// Each bounded draw is in `0..radius`; preserve vanilla's radius and XYZ
/// order.
fn sample_animation_blocks(
    player_pos: DVec3,
    mut next: impl FnMut(i32) -> i32,
    mut sample: impl FnMut(BlockPos),
) {
    let center = BlockPos::new(
        player_pos.x.floor() as i32,
        player_pos.y.floor() as i32,
        player_pos.z.floor() as i32,
    );
    for _ in 0..667 {
        for radius in [16, 32] {
            let x = center.x + next(radius) - next(radius);
            let y = center.y + next(radius) - next(radius);
            let z = center.z + next(radius) - next(radius);
            sample(BlockPos::new(x, y, z));
        }
    }
}
