//! Atlas-bound particle families and their shared construction and movement.

pub(crate) mod dust;
pub(crate) mod end_rod;
pub(crate) mod flame;
pub(crate) mod smoke;
pub(crate) mod terrain;

use end_rod::END_ROD_SPRITES;
use flame::FLAME_SPRITES;
use glam::{DVec3, dvec3};
use smoke::SMOKE_SPRITES;

use super::ParticleStore;
use super::state::ParticleState;
use crate::renderer::chunk::atlas::AtlasRegion;
use crate::world::chunk::ChunkStore;

/// Built-in particle sprites shared by initial atlas construction and reload.
pub fn sprite_names<'a>() -> impl Iterator<Item = &'a str> {
    END_ROD_SPRITES
        .into_iter()
        .chain(SMOKE_SPRITES)
        .chain(FLAME_SPRITES)
}

/// Explicit eager traversal after atlas availability, before any world tick.
pub(super) fn initialize(system: &mut ParticleStore) -> Result<(), super::RegistrationError> {
    for initialize in [
        smoke::initialize,
        flame::initialize,
        dust::initialize,
        terrain::initialize,
        end_rod::initialize,
    ] {
        initialize(system)?;
    }
    Ok(())
}

/// Shared smoke/dust movement, including acceleration before ground drag.
fn tick_rising(p: &mut ParticleState, chunks: &ChunkStore, frames: &[AtlasRegion; 8]) -> bool {
    if !p.begin_tick() {
        return false;
    }
    p.move_with_collision(chunks);
    if p.pos.y == p.prev_pos.y {
        p.vel.x *= 1.1;
        p.vel.z *= 1.1;
    }
    p.apply_drag();
    p.set_sprite(&frames[(p.age * 7 / p.lifetime) as usize]);
    true
}

fn growing_size(p: &ParticleState, partial_tick: f32) -> f32 {
    let age = (p.age as f32 + partial_tick) / p.lifetime as f32;
    p.size * (age * 32.0).clamp(0.0, 1.0)
}

/// Particle's float-randomized base-constructor motion.
pub(super) fn randomized_motion(velocity: DVec3, mut next: impl FnMut() -> f32) -> DVec3 {
    let mut jitter = || f64::from((next() * 2.0 - 1.0) * 0.4);
    let dir = velocity + dvec3(jitter(), jitter(), jitter());
    let speed = f64::from((next() + next() + 1.0) * 0.15);
    let mut vel = dir / dir.length() * speed * f64::from(0.4f32);
    vel.y += f64::from(0.1f32);
    vel
}
