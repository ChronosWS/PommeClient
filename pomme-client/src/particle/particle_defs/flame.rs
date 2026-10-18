use glam::{DVec3, dvec3};

use super::randomized_motion;
use super::smoke::smoke_flame_base;
use crate::particle::batch::{ParticleBehavior, ParticleEmitter, ParticleProvider};
use crate::particle::state::ParticleState;
use crate::particle::{ParticleStore, RegistrationError};
use crate::renderer::ParticleQuad;
use crate::renderer::chunk::atlas::{AtlasRegion, AtlasUVMap};
use crate::renderer::chunk::mesher::world_brightness_with_block_emission;
use crate::world::chunk::ChunkStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlameFlavor {
    Ordinary,
    Soul,
    Copper,
}

pub const FLAME_SPRITES: [&str; 3] = [
    "particle/flame",
    "particle/soul_fire_flame",
    "particle/copper_fire_flame",
];

pub(crate) struct FlameState {
    pub common: ParticleState,
    pub movement_origin: DVec3,
}

pub(crate) struct Flame {
    pub frames: [AtlasRegion; 3],
}

pub(super) fn initialize(system: &mut ParticleStore) -> Result<(), RegistrationError> {
    system.register(Flame::new(&system.uv_map))?;
    Ok(())
}

pub(crate) fn get_generator(
    system: &ParticleStore,
) -> Result<ParticleEmitter<Flame>, RegistrationError> {
    system.get_generator::<Flame>()
}

impl ParticleProvider for Flame {
    type Options = FlameFlavor;
    type State = FlameState;

    fn create(&self, flavor: FlameFlavor, pos: DVec3, velocity: DVec3) -> FlameState {
        let frame = &self.frames[match flavor {
            FlameFlavor::Ordinary => 0,
            FlameFlavor::Soul => 1,
            FlameFlavor::Copper => 2,
        }];
        // Particle's randomized motion, then RisingParticle's 1% contribution.
        let vel = randomized_motion(velocity, fastrand::f32) * f64::from(0.01f32) + velocity;
        let mut common = smoke_flame_base(pos, vel, frame, fastrand::f32());
        common.lifetime += 4;
        let jitter = || f64::from((fastrand::f32() - fastrand::f32()) * 0.05);
        common.pos += dvec3(jitter(), jitter(), jitter());
        FlameState {
            common,
            movement_origin: pos,
        }
    }
}

impl Flame {
    pub(in crate::particle) fn new(atlas: &AtlasUVMap) -> Self {
        Self {
            frames: FLAME_SPRITES.map(|name| atlas.get_region(name)),
        }
    }

    pub(in crate::particle) fn brightness(
        p: &ParticleState,
        chunks: &ChunkStore,
        partial_tick: f32,
    ) -> f32 {
        let age = (p.age as f32 + partial_tick) / p.lifetime as f32;
        world_brightness_with_block_emission(
            chunks,
            p.pos.x.floor() as i32,
            p.pos.y.floor() as i32,
            p.pos.z.floor() as i32,
            age.clamp(0.0, 1.0) * 240.0,
        )
    }
}

impl ParticleBehavior for Flame {
    type State = FlameState;

    fn tick(&self, state: &mut FlameState, _chunks: &ChunkStore) -> bool {
        let p = &mut state.common;
        if !p.begin_tick() {
            return false;
        }
        // The first movement starts at the unjittered box, not the visual pos.
        state.movement_origin += p.vel;
        p.pos = state.movement_origin;
        p.apply_drag();
        true
    }

    fn extract(
        &self,
        state: &FlameState,
        a: f32,
        anchor: DVec3,
        chunks: &ChunkStore,
    ) -> ParticleQuad {
        let p = &state.common;
        let age = (p.age as f32 + a) / p.lifetime as f32;
        p.quad(
            a,
            anchor,
            p.size * (1.0 - age * age * 0.5),
            Self::brightness(p, chunks, a),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::tests::{close, frames};
    use crate::world::block::find_state;

    #[test]
    fn flame_partial_shrink_and_lifetime_are_opaque() {
        let chunks = ChunkStore::with_origin_chunk();
        let handler = Flame {
            frames: [frames()[0]; 3],
        };
        let mut p = handler.create(FlameFlavor::Ordinary, DVec3::ZERO, DVec3::ZERO);
        p.common.lifetime = 20;
        let quad = handler.extract(&p, 0.5, DVec3::ZERO, &chunks);
        close(quad.size, p.common.size * (1.0 - 0.025 * 0.025 * 0.5));
        assert!(!quad.translucent);
        assert_eq!(quad.color.to_le_bytes()[3], 255);
        for _ in 0..20 {
            assert!(handler.tick(&mut p, &chunks));
        }
        close(
            handler.extract(&p, 0.0, DVec3::ZERO, &chunks).size,
            p.common.size * 0.5,
        );
        assert!(!handler.tick(&mut p, &chunks));
    }

    #[test]
    fn flame_first_tick_moves_from_unjittered_origin_and_keeps_sprite() {
        let chunks = ChunkStore::with_origin_chunk();
        chunks.set_block_state(2, 79, 4, find_state("stone", &[]));
        let handler = Flame {
            frames: [frames()[3]; 3],
        };
        let origin = dvec3(2.0, 80.0, 4.0);
        let mut state = handler.create(FlameFlavor::Ordinary, origin, DVec3::ZERO);
        assert_eq!(state.movement_origin, origin);
        let p = &mut state.common;
        assert_eq!(p.prev_pos, origin);
        assert!((p.pos - origin).abs().max_element() <= 0.05);
        let random_motion = p.vel - DVec3::Y * f64::from(0.1f32) * f64::from(0.01f32);
        assert!((0.000599..=0.001801).contains(&random_motion.length()));
        assert!((12..=44).contains(&p.lifetime));
        assert!((0.1..=0.2).contains(&p.size));
        p.pos = origin + dvec3(0.01, -0.02, 0.03);
        p.vel = dvec3(0.1, -0.2, 0.3);
        p.stopped_by_collision = true;
        let visual_pos = p.pos;
        let velocity = p.vel;
        assert!(handler.tick(&mut state, &chunks));
        let p = &state.common;
        assert_eq!(p.prev_pos, visual_pos);
        assert_eq!(p.pos, origin + velocity);
        assert_eq!(p.vel, velocity * f64::from(0.96f32));
        let first_pos = p.pos;
        let velocity = p.vel;
        assert!(handler.tick(&mut state, &chunks));
        let p = &state.common;
        assert_eq!(p.prev_pos, first_pos);
        assert_eq!(p.pos, first_pos + velocity);
        assert_eq!((p.u0, p.u1), (frames()[3].u_min, frames()[3].u_max));
    }
}
