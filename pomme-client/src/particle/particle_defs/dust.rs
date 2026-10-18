use glam::DVec3;

use super::smoke::SMOKE_SPRITES;
use super::{growing_size, randomized_motion, tick_rising};
use crate::particle::batch::{ParticleBehavior, ParticleEmitter, ParticleProvider};
use crate::particle::state::ParticleState;
use crate::particle::{ParticleStore, RegistrationError};
use crate::renderer::ParticleQuad;
use crate::renderer::chunk::atlas::{AtlasRegion, AtlasUVMap};
use crate::world::chunk::ChunkStore;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DustOptions {
    pub(in crate::particle) color: [f32; 3],
    pub(in crate::particle) scale: f32,
}

impl DustOptions {
    pub const fn redstone() -> Self {
        Self {
            color: [1.0, 0.0, 0.0],
            scale: 1.0,
        }
    }
}

pub(crate) struct Dust {
    pub frames: [AtlasRegion; 8],
}

pub(super) fn initialize(system: &mut ParticleStore) -> Result<(), RegistrationError> {
    system.register(Dust::new(&system.uv_map))?;
    Ok(())
}

pub(crate) fn get_generator(
    system: &ParticleStore,
) -> Result<ParticleEmitter<Dust>, RegistrationError> {
    system.get_generator::<Dust>()
}

impl Dust {
    pub(in crate::particle) fn new(atlas: &AtlasUVMap) -> Self {
        Self {
            frames: SMOKE_SPRITES.map(|name| atlas.get_region(name)),
        }
    }

    pub(in crate::particle) fn create_with_random(
        &self,
        options: DustOptions,
        pos: DVec3,
        velocity: DVec3,
        mut next_float: impl FnMut() -> f32,
        mut next_double: impl FnMut() -> f64,
    ) -> ParticleState {
        // The base lifetime is overwritten, but still consumes its float draw.
        let _ = next_float();
        let vel = randomized_motion(velocity, &mut next_float) * f64::from(0.1f32);
        let size = 0.1 * (next_float() * 0.5 + 0.5) * 2.0 * (0.75 * options.scale);
        let base_lifetime = (8.0 / (next_double() * 0.8 + 0.2)) as i32;
        let lifetime = (base_lifetime as f32 * options.scale).max(1.0) as i32;
        let base_factor = next_float() * 0.4 + 0.6;
        let color = options
            .color
            .map(|c| (next_float() * 0.2 + 0.8) * c * base_factor);
        let mut p = ParticleState {
            pos,
            prev_pos: pos,
            vel,
            age: 0,
            lifetime,
            on_ground: false,
            stopped_by_collision: false,
            gravity: 0.0,
            friction: f64::from(0.96f32),
            size,
            u0: 0.0,
            u1: 0.0,
            v0: 0.0,
            v1: 0.0,
            color,
            alpha: 1.0,
            light: 0.0,
        };
        p.set_sprite(&self.frames[0]);
        p
    }
}

impl ParticleProvider for Dust {
    type Options = DustOptions;
    type State = ParticleState;

    fn create(&self, options: DustOptions, pos: DVec3, velocity: DVec3) -> ParticleState {
        self.create_with_random(options, pos, velocity, fastrand::f32, fastrand::f64)
    }
}

impl ParticleBehavior for Dust {
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
    use crate::particle::tests::{close, dust, frames};
    use crate::world::block::find_state;

    #[test]
    fn dust_constructor_scales_random_motion_color_size_and_double_lifetime() {
        for (draw, size, red, motion) in [(0.0, 0.075, 0.48, -0.06), (0.75, 0.13125, 0.855, 0.15)] {
            // 0.15 rounded to f32 then widened would incorrectly produce 24.
            let p = dust(DVec3::ZERO, || draw, || 0.15);
            assert_eq!(p.lifetime, 25);
            let expected = (DVec3::ONE * (motion / 3.0f64.sqrt()) + DVec3::Y * 0.1) * 0.1;
            assert!((p.vel - expected).length() < 1e-8);
            close(p.size, size);
            close(p.color[0], red);
            assert_eq!(&p.color[1..], &[0.0, 0.0]);
            assert_eq!(p.gravity, 0.0);
        }
    }

    #[test]
    fn dust_growth_animation_and_expiry_are_opaque() {
        let chunks = ChunkStore::with_origin_chunk();
        let mut rng = fastrand::Rng::with_seed(4);
        let mut p = dust(dvec3(0.5, 80.0, 0.5), || rng.f32(), || 0.0625);
        p.vel = dvec3(0.01, 0.02, 0.03);
        let initial = (p.pos, p.vel);
        let handler = Dust { frames: frames() };
        close(growing_size(&p, 0.0), 0.0);
        close(
            handler.extract(&p, 0.5, DVec3::ZERO, &chunks).size,
            p.size * 0.5,
        );
        assert!(handler.tick(&mut p, &chunks));
        assert!((p.pos - initial.0 - initial.1).length() < 1e-12);
        assert_eq!(p.vel, initial.1 * f64::from(0.96f32));
        for _ in 1..32 {
            assert!(handler.tick(&mut p, &chunks));
        }
        assert_eq!(p.u0, frames()[7].u_min);
        let quad = handler.extract(&p, 0.5, DVec3::ZERO, &chunks);
        close(quad.size, p.size);
        assert!(!quad.translucent);
        assert_eq!(quad.color.to_le_bytes()[3], 255);
        assert!(!handler.tick(&mut p, &chunks));
    }

    #[test]
    fn dust_collides_and_speeds_up_when_vertical_motion_is_blocked() {
        let chunks = ChunkStore::with_origin_chunk();
        chunks.set_block_state(0, 0, 0, find_state("stone", &[]));
        chunks.set_block_state(0, 2, 0, find_state("stone", &[]));
        let handler = Dust { frames: frames() };
        for (y, vertical, ground) in [(1.0, -0.2, true), (1.8, 0.2, false)] {
            let mut rng = fastrand::Rng::with_seed(6);
            let mut p = dust(dvec3(0.5, y, 0.5), || rng.f32(), || 0.5);
            p.vel = dvec3(0.1, vertical, -0.1);
            assert!(handler.tick(&mut p, &chunks));
            assert_eq!(p.pos, dvec3(0.6, y, 0.4));
            assert_eq!(p.on_ground, ground);
            assert!(p.stopped_by_collision);
            let drag = if ground { 0.7 } else { 1.0 };
            assert_eq!(p.vel.x, 0.1 * 1.1 * f64::from(0.96f32) * drag);
            assert_eq!(p.vel.z, -0.1 * 1.1 * f64::from(0.96f32) * drag);
            assert_eq!(p.vel.y, vertical * f64::from(0.96f32));
            let stopped = p.pos;
            assert!(handler.tick(&mut p, &chunks));
            assert_eq!(p.pos, stopped);
        }
    }
}
