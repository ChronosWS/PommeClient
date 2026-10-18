use glam::DVec3;

use super::{growing_size, tick_rising};
use crate::particle::batch::{ParticleBehavior, ParticleEmitter, ParticleProvider};
use crate::particle::state::ParticleState;
use crate::particle::{ParticleStore, RegistrationError};
use crate::renderer::ParticleQuad;
use crate::renderer::chunk::atlas::{AtlasRegion, AtlasUVMap};
use crate::world::chunk::ChunkStore;

/// Smoke and dust both walk from generic_7 to generic_0.
pub const SMOKE_SPRITES: [&str; 8] = [
    "particle/generic_7",
    "particle/generic_6",
    "particle/generic_5",
    "particle/generic_4",
    "particle/generic_3",
    "particle/generic_2",
    "particle/generic_1",
    "particle/generic_0",
];

pub(crate) struct Smoke {
    pub frames: [AtlasRegion; 8],
}

pub(super) fn initialize(system: &mut ParticleStore) -> Result<(), RegistrationError> {
    system.register(Smoke::new(&system.uv_map))?;
    Ok(())
}

pub(crate) fn get_generator(
    system: &ParticleStore,
) -> Result<ParticleEmitter<Smoke>, RegistrationError> {
    system.get_generator::<Smoke>()
}

impl Smoke {
    pub(in crate::particle) fn new(atlas: &AtlasUVMap) -> Self {
        Self {
            frames: SMOKE_SPRITES.map(|name| atlas.get_region(name)),
        }
    }
}

/// Shared smoke/flame defaults. Keep intentional float lifetime arithmetic.
pub(super) fn smoke_flame_base(
    pos: DVec3,
    vel: DVec3,
    frame: &AtlasRegion,
    lifetime_random: f32,
) -> ParticleState {
    let mut p = ParticleState {
        pos,
        prev_pos: pos,
        vel,
        age: 0,
        lifetime: (8.0 / (lifetime_random * 0.8 + 0.2)) as i32,
        on_ground: false,
        stopped_by_collision: false,
        gravity: 0.0,
        friction: f64::from(0.96f32),
        size: 0.1 * (fastrand::f32() * 0.5 + 0.5) * 2.0,
        u0: 0.0,
        u1: 0.0,
        v0: 0.0,
        v1: 0.0,
        color: [1.0; 3],
        alpha: 1.0,
        light: 0.0,
    };
    p.set_sprite(frame);
    p
}

impl ParticleProvider for Smoke {
    type Options = ();
    type State = ParticleState;

    fn create(&self, (): (), pos: DVec3, vel: DVec3) -> ParticleState {
        let mut p = smoke_flame_base(pos, vel, &self.frames[0], fastrand::f32());
        p.gravity = f64::from(-0.1f32);
        p.size *= 0.75;
        p.color = [fastrand::f32() * 0.3; 3];
        p
    }
}

impl ParticleBehavior for Smoke {
    type State = ParticleState;

    fn tick(&self, p: &mut ParticleState, chunks: &ChunkStore) -> bool {
        tick_rising(p, chunks, &self.frames)
    }

    fn extract(
        &self,
        p: &ParticleState,
        a: f32,
        anchor: DVec3,
        chunks: &ChunkStore,
    ) -> ParticleQuad {
        p.quad(a, anchor, growing_size(p, a), p.world_light(chunks), false)
    }
}

#[cfg(test)]
mod tests {
    use glam::dvec3;

    use super::*;
    use crate::particle::particle_defs::flame::{Flame, FlameFlavor};
    use crate::particle::particle_defs::randomized_motion;
    use crate::particle::tests::{close, frames};
    use crate::world::block::find_state;

    #[test]
    fn smoke_and_flame_lifetimes_use_float_arithmetic() {
        let p = smoke_flame_base(DVec3::ZERO, DVec3::ZERO, &frames()[0], 0.15f32);
        assert_eq!(p.lifetime, 25);
        let handler = Flame {
            frames: [frames()[0]; 3],
        };
        fastrand::seed(42);
        let _ = randomized_motion(DVec3::ZERO, fastrand::f32);
        let base = smoke_flame_base(DVec3::ZERO, DVec3::ZERO, &frames()[0], fastrand::f32());
        fastrand::seed(42);
        let flame = handler.create(FlameFlavor::Ordinary, DVec3::ZERO, DVec3::ZERO);
        assert_eq!(flame.common.lifetime, base.lifetime + 4);
    }

    #[test]
    fn smoke_constructor_and_partial_growth_are_opaque() {
        let chunks = ChunkStore::with_origin_chunk();
        let handler = Smoke { frames: frames() };
        let velocity = dvec3(0.2, -0.1, 0.3);
        let mut p = handler.create((), dvec3(0.5, 80.0, 0.5), velocity);
        assert_eq!(p.vel, velocity);
        assert!((0.075..=0.15).contains(&p.size));
        assert!((8..=40).contains(&p.lifetime));
        assert!((0.0..0.3).contains(&p.color[0]));
        assert_eq!(p.color, [p.color[0]; 3]);
        p.lifetime = 32;
        let quad = handler.extract(&p, 0.5, DVec3::ZERO, &chunks);
        close(quad.size, p.size * 0.5);
        assert!(!quad.translucent);
        assert_eq!(quad.color.to_le_bytes()[3], 255);
        assert!(handler.tick(&mut p, &chunks));
        close(handler.extract(&p, 0.0, DVec3::ZERO, &chunks).size, p.size);
    }

    #[test]
    fn smoke_ticks_rise_damp_animate_and_expire() {
        crate::world::block::init("26.2");
        let chunks = ChunkStore::new(2);
        let handler = Smoke { frames: frames() };
        let mut p = handler.create((), dvec3(0.5, 80.0, 0.5), dvec3(0.1, 0.02, -0.1));
        p.lifetime = 14;
        assert_eq!(p.u0, frames()[0].u_min);
        let old_pos = p.pos;
        let movement = p.vel + DVec3::Y * (0.04 * f64::from(0.1f32));
        assert!(handler.tick(&mut p, &chunks));
        assert_eq!(p.prev_pos, old_pos);
        assert!((p.pos - old_pos - movement).length() < 1e-12);
        assert!((p.vel - movement * f64::from(0.96f32)).length() < 1e-12);
        assert_eq!(p.u0, frames()[0].u_min);
        for expected_frame in [1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7] {
            assert!(handler.tick(&mut p, &chunks));
            assert_eq!(p.u0, frames()[expected_frame].u_min);
            assert_eq!(p.u1, frames()[expected_frame].u_max);
        }
        let last_pos = p.pos;
        assert!(!handler.tick(&mut p, &chunks));
        assert_eq!(p.pos, last_pos);
    }

    #[test]
    fn smoke_collides_with_floor_before_acceleration_and_ground_drag() {
        let chunks = ChunkStore::with_origin_chunk();
        chunks.set_block_state(0, 0, 0, find_state("stone", &[]));
        let handler = Smoke { frames: frames() };
        let mut p = handler.create((), dvec3(0.5, 1.0, 0.5), dvec3(0.1, -0.2, -0.1));
        assert!(handler.tick(&mut p, &chunks));
        assert_eq!(p.pos, dvec3(0.6, 1.0, 0.4));
        assert!(p.on_ground);
        assert!(p.stopped_by_collision);
        assert_eq!(p.vel.x, 0.1 * 1.1 * f64::from(0.96f32) * 0.7);
        assert_eq!(p.vel.z, -0.1 * 1.1 * f64::from(0.96f32) * 0.7);
        let stopped_pos = p.pos;
        let stopped_velocity = p.vel;
        assert!(handler.tick(&mut p, &chunks));
        assert_eq!(p.pos, stopped_pos);
        assert_eq!(p.vel.x, stopped_velocity.x * 1.1 * f64::from(0.96f32) * 0.7);
        assert_eq!(p.vel.z, stopped_velocity.z * 1.1 * f64::from(0.96f32) * 0.7);
    }
}
