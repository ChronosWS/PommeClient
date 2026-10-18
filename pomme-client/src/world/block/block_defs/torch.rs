//! Floor and wall torch construction recipes and animation rules.

use azalea_block::BlockState;
use azalea_core::position::BlockPos;
use glam::{DVec3, dvec3};

use super::super::{Block, LevelContext, block_properties};
use super::BlockDefinitions;
use crate::particle::particle_defs::dust::{self, Dust, DustOptions};
use crate::particle::particle_defs::flame::{self, Flame, FlameFlavor};
use crate::particle::particle_defs::smoke::{self, Smoke};
use crate::particle::{ParticleEmitter, ParticleStore, RegistrationError};

pub(super) fn register_definitions(
    definitions: &mut BlockDefinitions,
    particles: &ParticleStore,
) -> Result<(), RegistrationError> {
    let smoke = smoke::get_generator(particles)?;
    let flame = flame::get_generator(particles)?;
    let dust = dust::get_generator(particles)?;
    for (floor, wall, flavor) in [
        ("torch", "wall_torch", FlameFlavor::Ordinary),
        ("soul_torch", "soul_wall_torch", FlameFlavor::Soul),
        ("copper_torch", "copper_wall_torch", FlameFlavor::Copper),
    ] {
        definitions.register(
            floor,
            Box::new(TorchBlock::new(smoke.clone(), flame.clone(), flavor)),
        );
        definitions.register(
            wall,
            Box::new(WallTorchBlock::new(smoke.clone(), flame.clone(), flavor)),
        );
    }
    definitions.register(
        "redstone_torch",
        Box::new(RedstoneTorchBlock::new(dust.clone())),
    );
    definitions.register(
        "redstone_wall_torch",
        Box::new(RedstoneWallTorchBlock::new(dust)),
    );
    Ok(())
}

struct TorchBlock {
    smoke: ParticleEmitter<Smoke>,
    flame: ParticleEmitter<Flame>,
    flavor: FlameFlavor,
}

impl TorchBlock {
    fn new(
        smoke: ParticleEmitter<Smoke>,
        flame: ParticleEmitter<Flame>,
        flavor: FlameFlavor,
    ) -> Self {
        Self {
            smoke,
            flame,
            flavor,
        }
    }
}

impl Block for TorchBlock {
    fn animate_tick(
        &self,
        _state: BlockState,
        pos: BlockPos,
        level: &mut LevelContext<'_>,
        _random: &mut fastrand::Rng,
    ) {
        emit_smoke_and_flame(
            &self.smoke,
            &self.flame,
            floor_anchor(pos),
            self.flavor,
            level,
        );
    }
}

struct WallTorchBlock {
    smoke: ParticleEmitter<Smoke>,
    flame: ParticleEmitter<Flame>,
    flavor: FlameFlavor,
}

impl WallTorchBlock {
    fn new(
        smoke: ParticleEmitter<Smoke>,
        flame: ParticleEmitter<Flame>,
        flavor: FlameFlavor,
    ) -> Self {
        Self {
            smoke,
            flame,
            flavor,
        }
    }
}

impl Block for WallTorchBlock {
    fn animate_tick(
        &self,
        state: BlockState,
        pos: BlockPos,
        level: &mut LevelContext<'_>,
        _random: &mut fastrand::Rng,
    ) {
        let Some(position) = wall_anchor(pos, block_properties(state).get("facing")) else {
            return;
        };
        emit_smoke_and_flame(&self.smoke, &self.flame, position, self.flavor, level);
    }
}

struct RedstoneTorchBlock {
    dust: ParticleEmitter<Dust>,
}

impl RedstoneTorchBlock {
    fn new(dust: ParticleEmitter<Dust>) -> Self {
        Self { dust }
    }
}

impl Block for RedstoneTorchBlock {
    fn animate_tick(
        &self,
        state: BlockState,
        pos: BlockPos,
        level: &mut LevelContext<'_>,
        random: &mut fastrand::Rng,
    ) {
        let lit = block_properties(state).get("lit") == Some("true");
        emit_redstone_dust(&self.dust, floor_anchor(pos), lit, level, random);
    }
}

struct RedstoneWallTorchBlock {
    dust: ParticleEmitter<Dust>,
}

impl RedstoneWallTorchBlock {
    fn new(dust: ParticleEmitter<Dust>) -> Self {
        Self { dust }
    }
}

impl Block for RedstoneWallTorchBlock {
    fn animate_tick(
        &self,
        state: BlockState,
        pos: BlockPos,
        level: &mut LevelContext<'_>,
        random: &mut fastrand::Rng,
    ) {
        let properties = block_properties(state);
        let lit = properties.get("lit") == Some("true");
        let Some(anchor) = wall_anchor(pos, properties.get("facing")) else {
            return;
        };
        emit_redstone_dust(&self.dust, anchor, lit, level, random);
    }
}

fn floor_anchor(pos: BlockPos) -> DVec3 {
    dvec3(
        f64::from(pos.x) + 0.5,
        f64::from(pos.y) + 0.7,
        f64::from(pos.z) + 0.5,
    )
}

fn wall_anchor(pos: BlockPos, facing: Option<&str>) -> Option<DVec3> {
    let (dx, dz) = match facing? {
        "north" => (0.0, 0.27),
        "south" => (0.0, -0.27),
        "west" => (0.27, 0.0),
        "east" => (-0.27, 0.0),
        _ => return None,
    };
    Some(dvec3(
        f64::from(pos.x) + 0.5 + dx,
        f64::from(pos.y) + 0.92,
        f64::from(pos.z) + 0.5 + dz,
    ))
}

fn emit_smoke_and_flame(
    smoke: &ParticleEmitter<Smoke>,
    flame: &ParticleEmitter<Flame>,
    position: DVec3,
    flavor: FlameFlavor,
    level: &mut LevelContext<'_>,
) {
    let result = smoke.create(&mut level.particles, (), position, DVec3::ZERO);
    if let Err(error) = result {
        tracing::error!(?error, "Torch smoke creation failed");
    }
    let result = flame.create(&mut level.particles, flavor, position, DVec3::ZERO);
    if let Err(error) = result {
        tracing::error!(?error, "Torch flame creation failed");
    }
}

fn emit_redstone_dust(
    dust: &ParticleEmitter<Dust>,
    anchor: DVec3,
    lit: bool,
    level: &mut LevelContext<'_>,
    random: &mut fastrand::Rng,
) {
    if !lit {
        return;
    }
    let position = anchor + dvec3(random.f64() - 0.5, random.f64() - 0.5, random.f64() - 0.5) * 0.2;
    let result = dust.create(
        &mut level.particles,
        DustOptions::redstone(),
        position,
        DVec3::ZERO,
    );
    if let Err(error) = result {
        tracing::error!(?error, "Redstone torch dust creation failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::tests::{assert_quad, empty_store};
    use crate::world::block::find_state;
    use crate::world::chunk::ChunkStore;

    fn anchors() -> [(Option<&'static str>, DVec3); 5] {
        [
            (None, dvec3(-1.5, 80.7, 3.5)),
            (Some("north"), dvec3(-1.5, 80.92, 3.77)),
            (Some("south"), dvec3(-1.5, 80.92, 3.23)),
            (Some("west"), dvec3(-1.23, 80.92, 3.5)),
            (Some("east"), dvec3(-1.77, 80.92, 3.5)),
        ]
    }

    #[test]
    fn torch_definitions_emit_smoke_then_flavored_flame_at_shared_anchors() {
        crate::world::block::init("26.2");
        let pos = BlockPos::new(-2, 80, 3);
        for (prefix, flavor) in [
            ("", FlameFlavor::Ordinary),
            ("soul_", FlameFlavor::Soul),
            ("copper_", FlameFlavor::Copper),
        ] {
            for (facing, anchor) in anchors() {
                let id = if facing.is_some() {
                    "wall_torch"
                } else {
                    "torch"
                };
                let properties: Vec<_> = facing.map(|f| ("facing", f)).into_iter().collect();
                let state = find_state(&format!("{prefix}{id}"), &properties);
                let mut store = empty_store();
                let definitions = BlockDefinitions::new(&store).unwrap();
                let mut random = fastrand::Rng::with_seed(42);
                fastrand::seed(25);
                definitions.get(state).animate_tick(
                    state,
                    pos,
                    &mut LevelContext {
                        particles: store.local_spawns(anchor),
                    },
                    &mut random,
                );
                assert_eq!(random.get_seed(), 42, "ordinary torches drew randomness");
                let next = fastrand::u64(..);
                let mut expected = empty_store();
                fastrand::seed(25);
                smoke::get_generator(&expected)
                    .unwrap()
                    .create(&mut expected.local_spawns(anchor), (), anchor, DVec3::ZERO)
                    .unwrap();
                flame::get_generator(&expected)
                    .unwrap()
                    .create(
                        &mut expected.local_spawns(anchor),
                        flavor,
                        anchor,
                        DVec3::ZERO,
                    )
                    .unwrap();
                assert_eq!(fastrand::u64(..), next, "smoke must construct before flame");
                let chunks = ChunkStore::with_origin_chunk();
                for _ in 0..2 {
                    store.tick(&chunks);
                    expected.tick(&chunks);
                    let actual = store.extract(0.5, anchor, &chunks);
                    let expected = expected.extract(0.5, anchor, &chunks);
                    assert_eq!(actual.len(), 2);
                    for (actual, expected) in actual.iter().zip(&expected) {
                        assert_quad(actual, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn redstone_gates_lit_and_jitters_xyz_at_floor_and_wall_anchors() {
        crate::world::block::init("26.2");
        let pos = BlockPos::new(-2, 80, 3);
        for (facing, anchor) in anchors() {
            let id = if facing.is_some() {
                "redstone_wall_torch"
            } else {
                "redstone_torch"
            };
            for lit in ["false", "true"] {
                let mut properties: Vec<_> = facing.map(|f| ("facing", f)).into_iter().collect();
                properties.push(("lit", lit));
                let state = find_state(id, &properties);
                assert_eq!(block_properties(state).get("lit"), Some(lit));
                let mut random = fastrand::Rng::with_seed(42);
                let mut expected = fastrand::Rng::with_seed(42);
                let mut store = empty_store();
                let definitions = BlockDefinitions::new(&store).unwrap();
                fastrand::seed(26);
                definitions.get(state).animate_tick(
                    state,
                    pos,
                    &mut LevelContext {
                        particles: store.local_spawns(anchor),
                    },
                    &mut random,
                );
                let chunks = ChunkStore::with_origin_chunk();
                store.tick(&chunks);
                if lit == "true" {
                    let jitter =
                        DVec3::from_array(std::array::from_fn(|_| (expected.f64() - 0.5) * 0.2));
                    let output = store.extract(0.0, anchor + jitter, &chunks);
                    assert_eq!(output.len(), 1);
                    assert_eq!(output[0].pos, [0.0; 3]);
                    assert_eq!(output[0].size, 0.0);
                    assert_eq!(&output[0].color.to_le_bytes()[1..3], &[0, 0]);
                    let next = fastrand::u64(..);
                    let mut reference = empty_store();
                    fastrand::seed(26);
                    dust::get_generator(&reference)
                        .unwrap()
                        .create(
                            &mut reference.local_spawns(anchor),
                            DustOptions::redstone(),
                            anchor + jitter,
                            DVec3::ZERO,
                        )
                        .unwrap();
                    assert_eq!(fastrand::u64(..), next);
                    reference.tick(&chunks);
                    reference.tick(&chunks);
                    store.tick(&chunks);
                    assert_quad(
                        &store.extract(0.5, anchor, &chunks)[0],
                        &reference.extract(0.5, anchor, &chunks)[0],
                    );
                } else {
                    assert!(store.extract(0.0, anchor, &chunks).is_empty());
                }
                assert_eq!(random.get_seed(), expected.get_seed());
            }
        }
    }

    #[test]
    fn wall_torch_rejects_missing_or_invalid_facing() {
        let pos = BlockPos::new(0, 0, 0);
        for facing in [None, Some("up"), Some("down"), Some("unknown")] {
            assert_eq!(wall_anchor(pos, facing), None);
        }
    }

    #[test]
    fn wall_definitions_without_facing_emit_nothing_and_draw_no_randomness() {
        crate::world::block::init("26.2");
        let state = find_state("redstone_torch", &[("lit", "true")]);
        let mut random = fastrand::Rng::with_seed(42);
        for id in ["wall_torch", "redstone_wall_torch"] {
            let mut store = empty_store();
            let definitions = BlockDefinitions::new(&store).unwrap();
            definitions.get(find_state(id, &[])).animate_tick(
                state,
                BlockPos::new(0, 0, 0),
                &mut LevelContext {
                    particles: store.local_spawns(DVec3::ZERO),
                },
                &mut random,
            );
            assert_eq!(random.get_seed(), 42);
            let chunks = ChunkStore::with_origin_chunk();
            store.tick(&chunks);
            assert!(store.extract(0.0, DVec3::ZERO, &chunks).is_empty());
        }
    }
}
