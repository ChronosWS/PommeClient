//! Runtime block definitions and particle bindings, with ambient-tick
//! orchestration.

use azalea_core::position::BlockPos;
use glam::DVec3;

use super::block::LevelContext;
use super::block::block_defs::BlockDefinitions;
use super::chunk::ChunkStore;
use crate::particle::{ParticleStore, RegistrationError};

/// Owns paired runtime block definitions and particles, and
/// persistent coordinate sampling randomness for vanilla `ClientLevel`/`Level`
/// ambient ticks. Chunks remain externally owned.
pub struct LevelRuntime {
    // Preserve vanilla's distribution and draw order, not its seeded sequence.
    animation_rng: fastrand::Rng,
    definitions: BlockDefinitions,
    particles: ParticleStore,
}

impl LevelRuntime {
    pub(crate) fn new(particles: ParticleStore) -> Result<Self, RegistrationError> {
        Ok(Self {
            animation_rng: fastrand::Rng::new(),
            definitions: BlockDefinitions::new(&particles)?,
            particles,
        })
    }

    /// Build resource-bound rules before replacing either member of the pair.
    /// Failure retains the old pair; neither outcome restarts coordinate
    /// sampling.
    pub(crate) fn rebind(&mut self, particles: ParticleStore) -> Result<(), RegistrationError> {
        let definitions = BlockDefinitions::new(&particles)?;
        self.definitions = definitions;
        self.particles = particles;
        Ok(())
    }

    pub(crate) fn particles(&self) -> &ParticleStore {
        &self.particles
    }

    /// Source adapters and the separately scheduled particle tick use this
    /// store. Resource replacement must go through `rebind` to rebuild the
    /// paired rules.
    pub(crate) fn particles_mut(&mut self) -> &mut ParticleStore {
        &mut self.particles
    }

    /// Vanilla `ClientLevel.animateTick`, called once per simulation tick.
    pub fn animate_tick(&mut self, chunks: &ChunkStore, player_pos: DVec3, camera_pos: DVec3) {
        self.definitions.refresh_protocol();
        animate_tick_with_draw(
            &self.definitions,
            chunks,
            player_pos,
            |radius| self.animation_rng.i32(0..radius),
            &mut LevelContext {
                particles: self.particles.local_spawns(camera_pos),
            },
            fastrand::Rng::new(),
        );
    }
}

/// Own the effect stream for this pass; all sampled callbacks borrow it.
fn animate_tick_with_draw(
    definitions: &BlockDefinitions,
    chunks: &ChunkStore,
    player_pos: DVec3,
    next: impl FnMut(i32) -> i32,
    level: &mut LevelContext<'_>,
    mut random: fastrand::Rng,
) {
    sample_animation_blocks(player_pos, next, |pos| {
        let state = chunks.get_block_state(pos.x, pos.y, pos.z);
        definitions
            .get(state)
            .animate_tick(state, pos, level, &mut random);
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

#[cfg(test)]
mod tests {
    use glam::dvec3;

    use super::*;
    use crate::particle::tests::empty_store;
    use crate::world::block::find_state;

    #[test]
    fn failed_rebind_retains_sampling_and_live_and_pending_particles() {
        use crate::particle::particle_defs::end_rod;
        let chunks = ChunkStore::with_origin_chunk();
        let mut runtime = LevelRuntime::new(empty_store()).unwrap();
        runtime.animation_rng = fastrand::Rng::with_seed(37);
        let emitter = end_rod::get_generator(runtime.particles()).unwrap();
        emitter
            .create(
                &mut runtime.particles_mut().local_spawns(DVec3::ZERO),
                (),
                DVec3::X,
                DVec3::ZERO,
            )
            .unwrap();
        runtime.particles_mut().tick(&chunks);
        emitter
            .create(
                &mut runtime.particles_mut().local_spawns(DVec3::ZERO),
                (),
                DVec3::Y,
                DVec3::ZERO,
            )
            .unwrap();
        assert!(matches!(
            runtime.rebind(crate::particle::tests::unregistered_store()),
            Err(RegistrationError::MissingDefinition(_))
        ));
        assert_eq!(runtime.animation_rng.get_seed(), 37);
        let output = runtime.particles().extract(0.0, DVec3::ZERO, &chunks);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].pos, [1.0, 0.0, 0.0]);
        // The original handle remains valid, and pending state was not lost.
        emitter
            .create(
                &mut runtime.particles_mut().local_spawns(DVec3::ZERO),
                (),
                DVec3::Z,
                DVec3::ZERO,
            )
            .unwrap();
        runtime.particles_mut().tick(&chunks);
        let output = runtime.particles().extract(0.0, DVec3::ZERO, &chunks);
        assert_eq!(output.len(), 3);
        assert_eq!(output[1].pos, [0.0, 1.0, 0.0]);
        assert_eq!(output[2].pos, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn reload_rebinds_resource_frames_without_resetting_sampling_randomness() {
        use crate::particle::particle_defs::flame::{self, FlameFlavor};
        use crate::particle::tests::{frames, store_with_frames};

        let chunks = ChunkStore::with_origin_chunk();
        let anchor = dvec3(0.5, 80.7, 0.5);
        let store = empty_store();
        let stale = flame::get_generator(&store).unwrap();
        let mut animation = LevelRuntime::new(store).unwrap();
        animation.animation_rng = fastrand::Rng::with_seed(17);
        animation.animate_tick(&chunks, anchor, anchor);
        let sampling_seed = animation.animation_rng.get_seed();
        assert_ne!(sampling_seed, 17);

        for offset in [0.0, 0.5] {
            if offset != 0.0 {
                let store = store_with_frames(frames().map(|mut frame| {
                    frame.u_min += offset;
                    frame.u_max += offset;
                    frame
                }));
                animation.rebind(store).unwrap();
                assert_eq!(animation.animation_rng.get_seed(), sampling_seed);
                assert_eq!(
                    stale.create(
                        &mut animation.particles_mut().local_spawns(anchor),
                        FlameFlavor::Ordinary,
                        anchor,
                        DVec3::ZERO
                    ),
                    Err(RegistrationError::WrongSystem)
                );
            }
            for (id, properties) in [
                ("torch", &[][..]),
                ("redstone_torch", &[("lit", "true")][..]),
            ] {
                let state = find_state(id, properties);
                animation.definitions.get(state).animate_tick(
                    state,
                    BlockPos::new(0, 80, 0),
                    &mut LevelContext {
                        particles: animation.particles.local_spawns(anchor),
                    },
                    &mut fastrand::Rng::with_seed(42),
                );
            }
            animation.particles_mut().tick(&chunks);
            let output = animation.particles().extract(0.0, anchor, &chunks);
            assert_eq!(output.len(), 3);
            for quad in output {
                assert_eq!(quad.u0, frames()[0].u_min + offset);
                assert_eq!(quad.u1, frames()[0].u_max + offset);
            }
        }
    }

    #[test]
    fn animation_sampler_preserves_count_radius_and_xyz_draw_order() {
        let mut draws = 0;
        let mut samples = Vec::new();
        sample_animation_blocks(
            dvec3(-0.1, -1.9, 2.9),
            |radius| {
                assert_eq!(radius, if (draws / 6) % 2 == 0 { 16 } else { 32 });
                let value = [0, radius - 1, radius - 1, 0, 3, 1][draws % 6];
                draws += 1;
                value
            },
            |pos| samples.push(pos),
        );
        assert_eq!(draws, 667 * 2 * 6);
        assert_eq!(samples.len(), 667 * 2);
        for pair in samples.as_chunks::<2>().0 {
            assert_eq!(pair[0], BlockPos::new(-16, 13, 4));
            assert_eq!(pair[1], BlockPos::new(-32, 29, 4));
        }
    }

    #[test]
    fn animation_sampler_persists_independently_of_dispatch_and_sink_randomness() {
        let filled = |id, properties: &[(&str, &str)]| {
            let chunks = ChunkStore::with_origin_chunk();
            let state = find_state(id, properties);
            for x in 0..16 {
                for y in 64..96 {
                    for z in 0..16 {
                        chunks.set_block_state(x, y, z, state);
                    }
                }
            }
            chunks
        };
        let torches = filled("torch", &[]);
        let redstone = filled("redstone_torch", &[("lit", "true")]);
        let air = ChunkStore::with_origin_chunk();
        let player_pos = dvec3(8.5, 80.5, 8.5);
        let mut animations: [LevelRuntime; 3] = std::array::from_fn(|_| {
            let mut animation = LevelRuntime::new(empty_store()).unwrap();
            animation.animation_rng = fastrand::Rng::with_seed(17);
            animation
        });
        let mut previous = Vec::new();
        for _ in 0..2 {
            animations[0].animate_tick(&redstone, player_pos, player_pos);
            animations[0].particles_mut().tick(&redstone);
            assert!(
                !animations[0]
                    .particles()
                    .extract(0.0, player_pos, &redstone)
                    .is_empty()
            );
            animations[1].animate_tick(&torches, player_pos, player_pos);
            animations[2].animate_tick(&air, player_pos, player_pos);
            animations[2].particles_mut().tick(&air);
            assert!(
                animations[2]
                    .particles()
                    .extract(0.0, player_pos, &air)
                    .is_empty()
            );
            let samples = animations.each_mut().map(|animation| {
                animation.rebind(empty_store()).unwrap();
                animation.animate_tick(&torches, player_pos, player_pos);
                animation.particles_mut().tick(&torches);
                animation
                    .particles()
                    .extract(0.0, player_pos, &torches)
                    .iter()
                    .map(|q| q.pos)
                    .collect::<Vec<_>>()
            });
            assert!(!samples[0].is_empty());
            assert_eq!(samples[0], samples[1]);
            assert_eq!(samples[0], samples[2]);
            assert_ne!(samples[0], previous, "sampling must advance between ticks");
            previous = samples[0].clone();
            for animation in &mut animations {
                animation.rebind(empty_store()).unwrap();
            }
        }
    }

    #[test]
    fn sampled_torches_stay_pending_until_store_tick() {
        let chunks = ChunkStore::with_origin_chunk();
        let player_pos = dvec3(8.5, 80.5, 8.5);
        // The separately tested source sampler supplies probes, never extracted
        // particles. Select three distinct loaded anchors so hits are guaranteed
        // without filling thousands of blocks or depending on random sampling.
        let mut random = fastrand::Rng::with_seed(17);
        let mut probes = Vec::new();
        sample_animation_blocks(player_pos, |r| random.i32(0..r), |p| probes.push(p));
        let mut fixtures = Vec::new();
        for &pos in &probes {
            if (0..16).contains(&pos.x)
                && (65..96).contains(&pos.y)
                && (0..16).contains(&pos.z)
                && !fixtures.contains(&pos)
            {
                fixtures.push(pos);
                if fixtures.len() == 3 {
                    break;
                }
            }
        }
        assert_eq!(fixtures.len(), 3);
        for pos in &fixtures {
            chunks.set_block_state(pos.x, pos.y, pos.z, find_state("torch", &[]));
        }
        let expected: Vec<_> = probes
            .iter()
            .filter(|pos| fixtures.contains(pos))
            .map(|pos| [pos.x as f32 + 0.5, pos.y as f32 + 0.7, pos.z as f32 + 0.5])
            .collect();
        assert!(expected.len() >= 3);
        let mut flame_frame = crate::particle::tests::frames()[3];
        flame_frame.v_min = 0.875;
        let mut animation = LevelRuntime::new(crate::particle::tests::store_with_regions([(
            "particle/flame",
            flame_frame,
        )]))
        .unwrap();
        animation.animation_rng = fastrand::Rng::with_seed(17);
        fastrand::seed(29);
        animation.animate_tick(&chunks, player_pos, player_pos);
        assert!(
            animation
                .particles()
                .extract(0.5, DVec3::ZERO, &chunks)
                .is_empty()
        );
        animation.particles_mut().tick(&chunks);
        let output = animation.particles().extract(0.0, DVec3::ZERO, &chunks);
        assert_eq!(output.len(), expected.len() * 2);
        for smoke in [true, false] {
            let family: Vec<_> = output.iter().filter(|q| (q.v0 != 0.875) == smoke).collect();
            assert_eq!(family.len(), expected.len());
            for (quad, position) in family.into_iter().zip(&expected) {
                assert_eq!(&quad.pos, position);
                if smoke {
                    assert_eq!(quad.size, 0.0, "activation must leave smoke at age zero");
                }
            }
        }
        animation.particles_mut().tick(&chunks);
        let output = animation.particles().extract(1.0, DVec3::ZERO, &chunks);
        assert_eq!(output.len(), expected.len() * 2);
        assert!(
            output
                .iter()
                .filter(|q| q.v0 != 0.875)
                .zip(&expected)
                .all(|(q, pos)| q.pos[1] > pos[1])
        );
    }

    #[test]
    fn effect_stream_is_shared_by_callbacks_and_owned_by_each_pass() {
        let chunks = ChunkStore::with_origin_chunk();
        chunks.set_block_state(0, 80, 0, find_state("redstone_torch", &[("lit", "true")]));
        let anchor = dvec3(0.5, 80.7, 0.5);
        // Identical initial streams prove each pass starts afresh, without relying
        // on distinct random seeds. The coordinator consumes each owned RNG.
        for _ in 0..2 {
            let mut expected = fastrand::Rng::with_seed(42);
            let mut store = empty_store();
            animate_tick_with_draw(
                &BlockDefinitions::new(&store).unwrap(),
                &chunks,
                dvec3(0.5, 80.5, 0.5),
                |_| 0,
                &mut LevelContext {
                    particles: store.local_spawns(anchor),
                },
                fastrand::Rng::with_seed(42),
            );
            store.tick(&chunks);
            let output = store.extract(0.0, anchor, &chunks);
            assert_eq!(output.len(), 667 * 2);
            for quad in output {
                let jitter =
                    DVec3::from_array(std::array::from_fn(|_| (expected.f64() - 0.5) * 0.2));
                assert_eq!(quad.pos, (anchor + jitter - anchor).as_vec3().to_array());
            }
        }
    }

    #[test]
    fn animation_samples_missing_chunks_and_outside_height_as_air() {
        let chunks = ChunkStore::with_origin_chunk();
        for player_pos in [
            dvec3(-100.5, 80.0, -100.5),
            dvec3(0.5, -1000.0, 0.5),
            dvec3(0.5, 1000.0, 0.5),
        ] {
            let mut store = empty_store();
            animate_tick_with_draw(
                &BlockDefinitions::new(&store).unwrap(),
                &chunks,
                player_pos,
                |_| 0,
                &mut LevelContext {
                    particles: store.local_spawns(player_pos),
                },
                fastrand::Rng::with_seed(42),
            );
            store.tick(&chunks);
            assert!(store.extract(0.0, player_pos, &chunks).is_empty());
        }
    }
}
