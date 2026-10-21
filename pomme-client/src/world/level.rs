//! Conceptual equivalent to Vanilla ClientLevel/Level.  Provides access to
//! resources other game systems need, without exposing internals of GameState
//! generally.
use azalea_block::BlockState;
use glam::DVec3;

use super::block::block_registry::BlockRegistry;
use super::chunk::ChunkStore;
use crate::position::BlockPos;

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

/// Owns persistent coordinate-sampling randomness for world animation.
/// Block rules, session protocol bindings and chunks remain externally owned.
pub struct LevelRuntime {
    // Preserve vanilla's distribution and draw order, not its seeded sequence.
    animation_rng: fastrand::Rng,
}

impl LevelRuntime {
    pub(crate) fn new() -> Self {
        Self {
            animation_rng: fastrand::Rng::new(),
        }
    }

    /// Vanilla `ClientLevel.animateTick`, called once per simulation tick.
    pub(crate) fn animate_tick(
        &mut self,
        block_registry: &BlockRegistry,
        chunks: &ChunkStore,
        player_pos: DVec3,
    ) {
        animate_tick_with_draw(
            block_registry,
            player_pos,
            |radius| self.animation_rng.i32(0..radius),
            &mut LevelContext::new(chunks),
            fastrand::Rng::new(),
        );
    }
}

/// Own the effect stream for this pass; all sampled callbacks borrow it.
fn animate_tick_with_draw(
    block_registry: &BlockRegistry,
    player_pos: DVec3,
    next: impl FnMut(i32) -> i32,
    level: &mut LevelContext<'_>,
    mut random: fastrand::Rng,
) {
    sample_animation_blocks(player_pos, next, |pos| {
        block_registry.animate_tick(pos, level, &mut random);
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
