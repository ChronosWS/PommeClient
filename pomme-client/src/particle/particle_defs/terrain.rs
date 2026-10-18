use glam::DVec3;

use crate::particle::batch::{ParticleBehavior, ParticleEmitter, ParticleProvider};
use crate::particle::state::ParticleState;
use crate::particle::{ParticleStore, RegistrationError};
use crate::renderer::ParticleQuad;
use crate::renderer::chunk::atlas::AtlasRegion;
use crate::world::chunk::ChunkStore;

#[derive(Clone, Copy)]
pub(in crate::particle) struct TerrainOptions {
    pub region: AtlasRegion,
    pub color: [f32; 3],
    pub light: f32,
}

pub(in crate::particle) struct Terrain;

pub(super) fn initialize(system: &mut ParticleStore) -> Result<(), RegistrationError> {
    let terrain = system.register(Terrain)?;
    system.register_shared(ItemDebris, &terrain)
}

pub(in crate::particle) fn get_generator(
    system: &ParticleStore,
) -> Result<ParticleEmitter<Terrain>, RegistrationError> {
    system.get_generator::<Terrain>()
}

pub(in crate::particle) fn get_item_generator(
    system: &ParticleStore,
) -> Result<ParticleEmitter<ItemDebris>, RegistrationError> {
    system.get_generator::<ItemDebris>()
}

impl ParticleProvider for Terrain {
    type Options = TerrainOptions;
    type State = ParticleState;

    fn create(&self, options: TerrainOptions, pos: DVec3, velocity: DVec3) -> ParticleState {
        self.create_terrain(options, pos, velocity)
    }
}

impl Terrain {
    fn create_terrain(
        &self,
        options: TerrainOptions,
        pos: DVec3,
        velocity: DVec3,
    ) -> ParticleState {
        let TerrainOptions {
            region,
            color,
            light,
        } = options;
        let jitter = || (fastrand::f64() * 2.0 - 1.0) * 0.4;
        let dir = velocity + DVec3::new(jitter(), jitter(), jitter());
        let speed = (fastrand::f64() + fastrand::f64() + 1.0) * 0.15;
        let mut vel = dir / dir.length() * speed * 0.4;
        vel.y += 0.1;
        let lifetime = (4.0 / (fastrand::f32() * 0.9 + 0.1)) as i32;
        let size = 0.1 * (fastrand::f32() * 0.5 + 0.5) * 2.0 / 2.0;
        // Quarter sub-tile; vanilla getU0/getU1 reverse the horizontal samples.
        let sprite_u = |f: f32| region.u_min + f * (region.u_max - region.u_min);
        let sprite_v = |f: f32| region.v_min + f * (region.v_max - region.v_min);
        let uo = fastrand::f32() * 3.0;
        let vo = fastrand::f32() * 3.0;
        ParticleState {
            pos,
            prev_pos: pos,
            vel,
            age: 0,
            lifetime,
            on_ground: false,
            stopped_by_collision: false,
            gravity: 1.0,
            friction: 0.98,
            size,
            u0: sprite_u((uo + 1.0) / 4.0),
            u1: sprite_u(uo / 4.0),
            v0: sprite_v(vo / 4.0),
            v1: sprite_v((vo + 1.0) / 4.0),
            color,
            alpha: 1.0,
            light,
        }
    }
}

pub(in crate::particle) struct ItemDebris;

impl ParticleProvider for ItemDebris {
    type Options = (AtlasRegion, f32);
    type State = ParticleState;

    fn create(&self, (region, light): Self::Options, pos: DVec3, velocity: DVec3) -> ParticleState {
        let mut p = Terrain.create_terrain(
            TerrainOptions {
                region,
                color: [1.0; 3],
                light,
            },
            pos,
            DVec3::ZERO,
        );
        p.vel = p.vel * 0.1 + velocity;
        p
    }
}

impl ParticleBehavior for Terrain {
    type State = ParticleState;

    fn tick(&self, p: &mut ParticleState, chunks: &ChunkStore) -> bool {
        if !p.begin_tick() {
            return false;
        }
        p.move_with_collision(chunks);
        p.apply_drag();
        p.light = p.world_light(chunks);
        true
    }

    fn extract(
        &self,
        p: &ParticleState,
        a: f32,
        anchor: DVec3,
        _chunks: &ChunkStore,
    ) -> ParticleQuad {
        p.quad(a, anchor, p.size, p.light, false)
    }
}

#[cfg(test)]
mod tests {
    use glam::dvec3;

    use super::*;
    use crate::particle::tests::{assert_quad, empty_store, frames};
    use crate::world::block::find_state;

    #[test]
    fn terrain_and_item_requests_route_to_shared_behavior() {
        let chunks = ChunkStore::with_origin_chunk();
        let position = dvec3(0.5, 80.0, 0.5);
        let velocity = dvec3(0.1, 0.2, -0.3);
        let terrain_options = TerrainOptions {
            region: frames()[2],
            color: [0.6, 0.3, 0.2],
            light: 0.5,
        };
        let item_options = (frames()[5], 0.75);
        for item in [false, true] {
            let mut store = empty_store();
            fastrand::seed(24);
            let mut expected = if item {
                ItemDebris.create(item_options, position, velocity)
            } else {
                Terrain.create(terrain_options, position, velocity)
            };
            let next = fastrand::u64(..);
            fastrand::seed(24);
            if item {
                get_item_generator(&store)
                    .unwrap()
                    .create(&mut store.source_spawns(), item_options, position, velocity)
                    .unwrap();
            } else {
                get_generator(&store)
                    .unwrap()
                    .create(
                        &mut store.source_spawns(),
                        terrain_options,
                        position,
                        velocity,
                    )
                    .unwrap();
            }
            assert_eq!(fastrand::u64(..), next);
            assert!(store.extract(0.0, DVec3::ZERO, &chunks).is_empty());
            store.tick(&chunks);
            for moved in [false, true] {
                if moved {
                    store.tick(&chunks);
                    assert!(Terrain.tick(&mut expected, &chunks));
                }
                let output = store.extract(0.5, DVec3::ZERO, &chunks);
                assert_eq!(output.len(), 1);
                assert_quad(
                    &output[0],
                    &Terrain.extract(&expected, 0.5, DVec3::ZERO, &chunks),
                );
            }
        }
    }

    #[test]
    fn terrain_uvs_are_quarter_tiles_with_reversed_horizontal_orientation() {
        fastrand::seed(38);
        for _ in 0..32 {
            let p = Terrain.create(
                TerrainOptions {
                    region: frames()[2],
                    color: [1.0; 3],
                    light: 1.0,
                },
                DVec3::ZERO,
                DVec3::ZERO,
            );
            // Literal fixture bounds: U [0.25, 0.375], V [0.25, 0.75].
            assert!((0.25..=0.375).contains(&p.u0));
            assert!((0.25..=0.375).contains(&p.u1));
            assert!((0.25..=0.75).contains(&p.v0));
            assert!((0.25..=0.75).contains(&p.v1));
            crate::particle::tests::close(p.u0 - p.u1, 0.03125);
            crate::particle::tests::close(p.v1 - p.v0, 0.125);
        }
    }

    #[test]
    fn terrain_collision_preserves_sprite_and_opaque_layer() {
        let chunks = ChunkStore::with_origin_chunk();
        chunks.set_block_state(0, 0, 0, find_state("stone", &[]));
        let mut p = Terrain.create_terrain(
            TerrainOptions {
                region: frames()[2],
                color: [0.6; 3],
                light: 0.5,
            },
            dvec3(0.5, 1.0, 0.5),
            DVec3::ZERO,
        );
        let uv = (p.u0, p.u1, p.v0, p.v1);
        p.vel = dvec3(0.1, -0.2, -0.1);
        assert!(Terrain.tick(&mut p, &chunks));
        assert_eq!(p.pos, dvec3(0.6, 1.0, 0.4));
        assert_eq!(p.vel.x, 0.1 * 0.98 * 0.7);
        assert_eq!(uv, (p.u0, p.u1, p.v0, p.v1));
        assert!(!Terrain.extract(&p, 0.0, DVec3::ZERO, &chunks).translucent);
    }
}
