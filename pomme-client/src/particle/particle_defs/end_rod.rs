use glam::DVec3;

use crate::particle::batch::{ParticleBehavior, ParticleEmitter, ParticleProvider};
use crate::particle::state::ParticleState;
use crate::particle::{ParticleStore, RegistrationError};
use crate::renderer::ParticleQuad;
use crate::renderer::chunk::atlas::{AtlasRegion, AtlasUVMap};
use crate::world::chunk::ChunkStore;

/// Frame order from `assets/minecraft/particles/end_rod.json`.
pub const END_ROD_SPRITES: [&str; 8] = [
    "particle/glitter_7",
    "particle/glitter_6",
    "particle/glitter_5",
    "particle/glitter_4",
    "particle/glitter_3",
    "particle/glitter_2",
    "particle/glitter_1",
    "particle/glitter_0",
];

pub(crate) struct EndRod {
    pub frames: [AtlasRegion; 8],
}

pub(super) fn initialize(system: &mut ParticleStore) -> Result<(), RegistrationError> {
    system.register(EndRod::new(&system.uv_map))?;
    Ok(())
}

pub(crate) fn get_generator(
    system: &ParticleStore,
) -> Result<ParticleEmitter<EndRod>, RegistrationError> {
    system.get_generator::<EndRod>()
}

impl EndRod {
    pub(in crate::particle) fn new(atlas: &AtlasUVMap) -> Self {
        Self {
            frames: END_ROD_SPRITES.map(|name| atlas.get_region(name)),
        }
    }
}

impl ParticleProvider for EndRod {
    type Options = ();
    type State = ParticleState;

    fn create(&self, (): (), pos: DVec3, vel: DVec3) -> ParticleState {
        let size = 0.1 * (fastrand::f32() * 0.5 + 0.5) * 2.0 * 0.75;
        let mut p = ParticleState {
            pos,
            prev_pos: pos,
            vel,
            age: 0,
            lifetime: 60 + fastrand::i32(0..12),
            on_ground: false,
            stopped_by_collision: false,
            gravity: f64::from(0.0125f32),
            friction: f64::from(0.91f32),
            size,
            u0: 0.0,
            u1: 0.0,
            v0: 0.0,
            v1: 0.0,
            color: [1.0; 3],
            alpha: 1.0,
            // SimpleAnimatedParticle.getLightCoords is always full-bright.
            light: 1.0,
        };
        p.set_sprite(&self.frames[0]);
        p
    }
}

impl ParticleBehavior for EndRod {
    type State = ParticleState;

    fn tick(&self, p: &mut ParticleState, _chunks: &ChunkStore) -> bool {
        if !p.begin_tick() {
            return false;
        }
        p.pos += p.vel;
        p.apply_drag();
        p.set_sprite(&self.frames[(p.age * 7 / p.lifetime) as usize]);
        if p.age > p.lifetime / 2 {
            p.alpha = 1.0 - (p.age - p.lifetime / 2) as f32 / p.lifetime as f32;
            // SimpleAnimatedParticle.setFadeColor(0xF2DEC9).
            for (c, f) in p
                .color
                .iter_mut()
                .zip([242.0 / 255.0, 222.0 / 255.0, 201.0 / 255.0])
            {
                *c += (f - *c) * 0.2;
            }
        }
        true
    }

    fn extract(
        &self,
        p: &ParticleState,
        a: f32,
        anchor: DVec3,
        _chunks: &ChunkStore,
    ) -> ParticleQuad {
        p.quad(a, anchor, p.size, p.light, true)
    }
}

#[cfg(test)]
mod tests {
    use glam::dvec3;

    use super::*;
    use crate::particle::tests::{close, frames};

    #[test]
    fn end_rod_fades_animates_and_interpolates_before_narrowing() {
        let chunks = ChunkStore::with_origin_chunk();
        let handler = EndRod { frames: frames() };
        let anchor = dvec3(30_000_000.0, 80.0, -30_000_000.0);
        let mut p = handler.create((), anchor + DVec3::X * 0.125, dvec3(0.25, 0.0, 0.0));
        p.lifetime = 4;
        let initial = p.pos;
        assert!(handler.tick(&mut p, &chunks));
        let quad = handler.extract(&p, 0.5, anchor, &chunks);
        assert_eq!(
            quad.pos,
            (initial.lerp(p.pos, 0.5) - anchor).as_vec3().to_array()
        );
        assert!(quad.translucent);
        assert_eq!(quad.color, u32::MAX);
        assert_eq!(p.u0, frames()[1].u_min);
        for _ in 1..3 {
            assert!(handler.tick(&mut p, &chunks));
        }
        close(p.alpha, 0.75);
        close(p.color[0], 1.0 + (242.0 / 255.0 - 1.0) * 0.2);
        assert_eq!(p.u0, frames()[5].u_min);
        assert!(handler.tick(&mut p, &chunks));
        assert!(!handler.tick(&mut p, &chunks));
    }
}
