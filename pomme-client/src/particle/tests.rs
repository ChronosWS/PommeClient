use azalea_core::position::BlockPos;
use glam::dvec3;

use super::batch::{ParticleBehavior, ParticleProvider};
use super::particle_defs::dust::{Dust, DustOptions};
use super::particle_defs::end_rod::{END_ROD_SPRITES, EndRod};
use super::particle_defs::flame::{FLAME_SPRITES, Flame, FlameFlavor};
use super::particle_defs::smoke::{SMOKE_SPRITES, Smoke};
use super::particle_defs::terrain::{ItemDebris, Terrain, TerrainOptions};
use super::state::ParticleState;
use super::*;
use crate::renderer::chunk::atlas::AtlasRegion;
use crate::world::block::block_defs::BlockDefinitions;
use crate::world::block::{LevelContext, find_state};
use crate::world::chunk::ChunkLightData;

pub(crate) fn frames() -> [AtlasRegion; 8] {
    std::array::from_fn(|i| AtlasRegion {
        u_min: i as f32 / 8.0,
        u_max: (i + 1) as f32 / 8.0,
        v_min: 0.25,
        v_max: 0.75,
        pixel_rect: [0, 0, 16, 16],
        sprite: i as u16,
        opaque: false,
        translucent: false,
    })
}

pub(super) fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

pub(crate) fn empty_store() -> ParticleStore {
    store_with_frames(frames())
}

pub(crate) fn unregistered_store() -> ParticleStore {
    let colormap = Arc::new(Colormap::constant([255; 3]));
    ParticleStore::initialize(
        AtlasUVMap::with_regions(frames()[7], []),
        Arc::clone(&colormap),
        Arc::clone(&colormap),
        colormap,
        |_| Ok(()),
    )
    .unwrap()
}

pub(crate) fn store_with_frames(frames: [AtlasRegion; 8]) -> ParticleStore {
    let regions = SMOKE_SPRITES
        .into_iter()
        .zip(frames)
        .chain(END_ROD_SPRITES.into_iter().zip(frames))
        .chain(FLAME_SPRITES.into_iter().zip(frames));
    store_with_regions(regions)
}

pub(crate) fn store_with_regions<'a>(
    regions: impl IntoIterator<Item = (&'a str, AtlasRegion)>,
) -> ParticleStore {
    let colormap = Arc::new(Colormap::constant([255; 3]));
    ParticleStore::new(
        AtlasUVMap::with_regions(
            frames()[7],
            regions
                .into_iter()
                .map(|(name, frame)| (name.to_owned(), frame)),
        ),
        Arc::clone(&colormap),
        Arc::clone(&colormap),
        colormap,
    )
    .unwrap()
}

#[test]
fn builtin_initialization_propagates_registration_failure() {
    let colormap = Arc::new(Colormap::constant([255; 3]));
    let mut identity = Weak::new();
    let result = ParticleStore::initialize(
        AtlasUVMap::with_regions(frames()[7], []),
        Arc::clone(&colormap),
        Arc::clone(&colormap),
        colormap,
        |store| {
            identity = Rc::downgrade(&store.identity);
            store.register(Smoke::new(&store.uv_map))?;
            // Exercise the real eager traversal, with a programming error in
            // registration, not a missing texture or a deleted binding.
            let result = particle_defs::initialize(store);
            assert!(
                matches!(
                    store.get_generator::<Flame>(),
                    Err(RegistrationError::MissingDefinition(_))
                ),
                "initialization must stop at the first error"
            );
            result
        },
    );
    assert!(matches!(result,
        Err(RegistrationError::DuplicateDefinition(name))
            if name == std::any::type_name::<Smoke>()));
    assert!(
        identity.upgrade().is_none(),
        "partial store must be dropped"
    );
}

#[test]
fn block_initialization_reports_missing_bindings_and_rebind_is_atomic() {
    fastrand::seed(17);
    let original = empty_store();
    let original_identity = Rc::downgrade(&original.identity);
    let mut animation = crate::world::level::LevelRuntime::new(original).unwrap();
    for (key, name) in [
        (TypeId::of::<Smoke>(), std::any::type_name::<Smoke>()),
        (TypeId::of::<Flame>(), std::any::type_name::<Flame>()),
        (TypeId::of::<Dust>(), std::any::type_name::<Dust>()),
    ] {
        let mut incomplete = unregistered_store();
        if key != TypeId::of::<Smoke>() {
            incomplete.register(Smoke::new(&incomplete.uv_map)).unwrap();
        }
        if key != TypeId::of::<Flame>() {
            incomplete.register(Flame::new(&incomplete.uv_map)).unwrap();
        }
        if key != TypeId::of::<Dust>() {
            incomplete.register(Dust::new(&incomplete.uv_map)).unwrap();
        }
        assert!(
            matches!(BlockDefinitions::new(&incomplete), Err(RegistrationError::MissingDefinition(missing)) if missing == name)
        );
        assert_eq!(
            animation.rebind(incomplete),
            Err(RegistrationError::MissingDefinition(name))
        );
        assert!(original_identity.ptr_eq(&Rc::downgrade(&animation.particles().identity)));
    }
    let chunks = ChunkStore::with_origin_chunk();
    let state = find_state("torch", &[]);
    for x in 0..16 {
        for y in 64..96 {
            for z in 0..16 {
                chunks.set_block_state(x, y, z, state);
            }
        }
    }
    let position = dvec3(8.5, 80.5, 8.5);
    animation.animate_tick(&chunks, position, position);
    assert!(
        animation.particles().len() > 0,
        "failed rebind must retain the original definitions"
    );
}

#[test]
fn block_rules_use_injected_handles_without_resolving_bindings_again() {
    let mut store = empty_store();
    let definitions = BlockDefinitions::new(&store).unwrap();
    store.bindings.clear();
    let mut expected = 0;
    for id in [
        "torch",
        "wall_torch",
        "soul_torch",
        "soul_wall_torch",
        "copper_torch",
        "copper_wall_torch",
        "redstone_torch",
        "redstone_wall_torch",
    ] {
        let mut properties = Vec::new();
        if id.contains("wall") {
            properties.push(("facing", "north"));
        }
        let redstone = id.starts_with("redstone");
        if redstone {
            properties.push(("lit", "true"));
        }
        let state = find_state(id, &properties);
        definitions.get(state).animate_tick(
            state,
            BlockPos::new(0, 0, 0),
            &mut LevelContext {
                particles: store.local_spawns(DVec3::ZERO),
            },
            &mut fastrand::Rng::with_seed(42),
        );
        expected += if redstone { 1 } else { 2 };
        assert_eq!(
            store.len(),
            expected,
            "{id} must use its injected generators"
        );
    }
}

#[test]
fn builtin_bindings_use_literal_asset_regions_and_missing_sprites_fall_back() {
    // Independent resource keys: changing a production sprite constant must
    // break the matching assertion, not silently update the fixture too.
    let mut store = store_with_regions([
        ("particle/generic_7", frames()[0]),
        ("particle/glitter_7", frames()[1]),
        ("particle/flame", frames()[2]),
        ("particle/soul_fire_flame", frames()[3]),
        ("particle/copper_fire_flame", frames()[4]),
    ]);
    let mut context = store.source_spawns();
    spawn::<Smoke>(&mut context, (), DVec3::ZERO, DVec3::ZERO);
    spawn::<Dust>(&mut context, DustOptions::redstone(), DVec3::X, DVec3::ZERO);
    spawn::<EndRod>(&mut context, (), DVec3::X * 2.0, DVec3::ZERO);
    for (i, flavor) in [
        FlameFlavor::Ordinary,
        FlameFlavor::Soul,
        FlameFlavor::Copper,
    ]
    .into_iter()
    .enumerate()
    {
        spawn::<Flame>(&mut context, flavor, DVec3::X * (i + 3) as f64, DVec3::ZERO);
    }
    let chunks = ChunkStore::with_origin_chunk();
    store.tick(&chunks);
    let mut output = store.extract(0.0, DVec3::ZERO, &chunks);
    output.sort_by(|a, b| a.pos[0].total_cmp(&b.pos[0]));
    assert_eq!(output.len(), 6);
    for (quad, frame) in output.iter().zip([0, 0, 1, 2, 3, 4]) {
        assert_eq!(
            (quad.u0, quad.u1),
            (frames()[frame].u_min, frames()[frame].u_max)
        );
    }
    // Missing resources are valid atlas fallbacks, not registration failures.
    let mut fallback = store_with_regions([]);
    spawn::<Flame>(
        &mut fallback.source_spawns(),
        FlameFlavor::Soul,
        DVec3::ZERO,
        DVec3::ZERO,
    );
    fallback.tick(&chunks);
    assert_eq!(
        fallback.extract(0.0, DVec3::ZERO, &chunks)[0].u0,
        frames()[7].u_min
    );
}

pub(super) fn dust(
    pos: DVec3,
    next_float: impl FnMut() -> f32,
    next_double: impl FnMut() -> f64,
) -> ParticleState {
    Dust { frames: frames() }.create_with_random(
        DustOptions::redstone(),
        pos,
        DVec3::ZERO,
        next_float,
        next_double,
    )
}

pub(crate) fn assert_quad(actual: &ParticleQuad, expected: &ParticleQuad) {
    assert_eq!(actual.pos, expected.pos);
    close(actual.size, expected.size);
    assert_eq!(
        (actual.u0, actual.u1, actual.v0, actual.v1),
        (expected.u0, expected.u1, expected.v0, expected.v1),
    );
    assert_eq!(actual.color, expected.color);
    assert_eq!(actual.translucent, expected.translucent);
}

/// Routing oracle only: the registered boundary must use the bound provider
/// and behavior. Independent lifecycle/formula tests live in each definition.
fn assert_registered_matches<B>(handler: B, options: B::Options, position: DVec3, velocity: DVec3)
where
    B: ParticleBehavior + ParticleProvider<State = <B as ParticleBehavior>::State> + 'static,
    B::Options: Copy,
{
    let chunks = ChunkStore::with_origin_chunk();
    let mut store = empty_store();
    fastrand::seed(17);
    let mut state = handler.create(options, position, velocity);
    let next = fastrand::u64(..);
    fastrand::seed(17);
    spawn::<B>(
        &mut store.local_spawns(position),
        options,
        position,
        velocity,
    );
    assert_eq!(fastrand::u64(..), next);
    assert_eq!(store.len(), 1);
    assert!(store.extract(0.5, DVec3::ZERO, &chunks).is_empty());
    store.tick(&chunks);
    for moved in [false, true] {
        if moved {
            store.tick(&chunks);
            assert!(handler.tick(&mut state, &chunks));
        }
        let output = store.extract(0.5, DVec3::ZERO, &chunks);
        assert_eq!(output.len(), 1);
        assert_quad(
            &output[0],
            &handler.extract(&state, 0.5, DVec3::ZERO, &chunks),
        );
    }
}

#[test]
fn redstone_cutoff_uses_jittered_spawn_and_still_consumes_three_draws() {
    crate::world::block::init("26.2");
    let pos = BlockPos::new(0, 80, 0);
    for (id, anchor) in [
        ("redstone_torch", dvec3(0.5, 80.7, 0.5)),
        ("redstone_wall_torch", dvec3(0.5, 80.92, 0.77)),
    ] {
        let properties = if id == "redstone_torch" {
            &[("lit", "true")][..]
        } else {
            &[("lit", "true"), ("facing", "north")][..]
        };
        let state = find_state(id, properties);
        let mut expected = fastrand::Rng::with_seed(42);
        let spawn =
            anchor + DVec3::from_array(std::array::from_fn(|_| (expected.f64() - 0.5) * 0.2));
        for (distance, count) in [(31.999, 1), (32.0, 1), (32.001, 0)] {
            let mut store = empty_store();
            let definitions = BlockDefinitions::new(&store).unwrap();
            let mut random = fastrand::Rng::with_seed(42);
            // An anchor-based cull would reject this boundary.
            let direction = (spawn.z - anchor.z).signum();
            definitions.get(state).animate_tick(
                state,
                pos,
                &mut LevelContext {
                    particles: store.local_spawns(spawn + DVec3::Z * distance * direction),
                },
                &mut random,
            );
            assert_eq!(store.len(), count, "{id} at {distance}");
            assert_eq!(random.get_seed(), expected.get_seed());
            if count != 0 {
                let chunks = ChunkStore::with_origin_chunk();
                assert!(store.extract(0.0, spawn, &chunks).is_empty());
                store.tick(&chunks);
                assert_eq!(store.extract(0.0, spawn, &chunks)[0].pos, [0.0; 3]);
            }
        }
    }
}

#[test]
fn local_requests_route_smoke_and_selected_flame_sprites() {
    let expected = dvec3(-1.5, 80.7, 3.5);
    for (flavor, frame) in [
        (FlameFlavor::Ordinary, 0),
        (FlameFlavor::Soul, 1),
        (FlameFlavor::Copper, 2),
    ] {
        let mut store = empty_store();
        spawn::<Smoke>(
            &mut store.local_spawns(expected),
            (),
            expected + DVec3::X,
            DVec3::ZERO,
        );
        spawn::<Flame>(
            &mut store.local_spawns(expected),
            flavor,
            expected,
            DVec3::ZERO,
        );
        assert_eq!(store.len(), 2);
        let chunks = ChunkStore::with_origin_chunk();
        store.tick(&chunks);
        let output = store.extract(0.0, expected, &chunks);
        let smoke = output.iter().find(|q| q.pos == [1.0, 0.0, 0.0]).unwrap();
        let flame = output.iter().find(|q| q.pos == [0.0; 3]).unwrap();
        assert_eq!(smoke.u0, frames()[0].u_min);
        assert_eq!(
            (flame.u0, flame.u1),
            (frames()[frame].u_min, frames()[frame].u_max)
        );
    }
}

#[test]
fn torch_camera_cutoff_uses_exact_spawn_and_includes_boundary() {
    crate::world::block::init("26.2");
    let pos = BlockPos::new(0, 80, 0);
    for prefix in ["", "soul_", "copper_"] {
        for (id, properties, spawn) in [
            ("torch", &[][..], dvec3(0.5, 80.7, 0.5)),
            (
                "wall_torch",
                &[("facing", "east")][..],
                dvec3(0.23, 80.92, 0.5),
            ),
        ] {
            let state = find_state(&format!("{prefix}{id}"), properties);
            for (distance, count) in [(31.999, 2), (32.0, 2), (32.001, 0)] {
                let mut store = empty_store();
                let definitions = BlockDefinitions::new(&store).unwrap();
                let mut random = fastrand::Rng::with_seed(42);
                definitions.get(state).animate_tick(
                    state,
                    pos,
                    &mut LevelContext {
                        particles: store.local_spawns(spawn + DVec3::Z * distance),
                    },
                    &mut random,
                );
                assert_eq!(store.len(), count, "{id} at {distance}");
                assert_eq!(random.get_seed(), 42);
            }
        }
    }
}

#[test]
fn smoke_and_flame_requests_route_to_bound_providers() {
    let pos = dvec3(0.5, 80.0, 0.5);
    let velocity = dvec3(0.2, -0.1, 0.3);
    assert_registered_matches(Smoke { frames: frames() }, (), pos, velocity);
    assert_registered_matches(
        Flame {
            frames: [frames()[0], frames()[1], frames()[2]],
        },
        FlameFlavor::Ordinary,
        pos,
        velocity,
    );
}

#[test]
fn particles_sample_current_light_and_flame_adds_partial_age_block_emission() {
    let lit_chunks = |sky: u8, block: u8| {
        let mut chunks = ChunkStore::new(2);
        chunks.light_data.insert(
            (0, 0),
            Arc::new(ChunkLightData {
                sky_sections: vec![None, Some(Box::new([sky * 17; 2048]))],
                block_sections: vec![None, Some(Box::new([block * 17; 2048]))],
                min_y: 0,
                has_sky: true,
                sky_top_section: Some(2),
            }),
        );
        chunks
    };
    let chunks = lit_chunks(0, 0);
    let pos = dvec3(0.5, 0.5, 0.5);
    for mut p in [
        Smoke { frames: frames() }.create((), pos, DVec3::ZERO),
        dust(pos, || 0.75, || 0.15),
    ] {
        close(p.world_light(&chunks), 0.05);
        p.pos.x = -0.5;
        close(p.world_light(&chunks), 1.0);
    }
    let mut state = Flame {
        frames: [frames()[0]; 3],
    }
    .create(FlameFlavor::Ordinary, pos, DVec3::ZERO);
    let flame = &mut state.common;
    flame.pos = pos;
    flame.prev_pos = dvec3(-0.5, 0.5, 0.5);
    flame.lifetime = 20;
    close(Flame::brightness(flame, &chunks, 0.0), 0.05);
    flame.age = 20;
    close(Flame::brightness(flame, &chunks, 1.0), 1.0);
    flame.age = 9;
    // Partial age adds 114 packed block-light units (level 7.125), not sky light.
    for (sky, block, expected) in [
        (0, 0, 0.227 + (0.272 - 0.227) * 0.125),
        (12, 0, 0.566),
        (0, 2, 0.328 + (0.393 - 0.328) * 0.125),
        (12, 10, 1.0),
        (15, 0, 1.0),
    ] {
        close(
            Flame::brightness(flame, &lit_chunks(sky, block), 0.5),
            expected,
        );
    }
}

#[test]
fn requests_forward_velocity_and_dust_options_to_bound_providers() {
    let position = dvec3(1.0, 80.0, 2.0);
    let velocity = dvec3(0.3, 0.2, -0.1);
    // Internal options exercise the factory without adding a public validation API.
    let options = DustOptions {
        color: [0.2, 0.5, 0.8],
        scale: 2.0,
    };
    assert_registered_matches(
        Flame {
            frames: [frames()[0], frames()[1], frames()[2]],
        },
        FlameFlavor::Soul,
        position,
        velocity,
    );
    assert_registered_matches(Dust { frames: frames() }, options, position, velocity);
    assert_registered_matches(EndRod { frames: frames() }, (), position, velocity);
    let handler = Dust { frames: frames() };
    let redstone = handler.create_with_random(
        DustOptions::redstone(),
        position,
        velocity,
        || 0.75,
        || 0.15,
    );
    let colored = handler.create_with_random(options, position, velocity, || 0.75, || 0.15);
    assert_eq!(colored.vel, redstone.vel);
    assert_eq!(colored.lifetime, redstone.lifetime * 2);
    close(colored.size, redstone.size * 2.0);
    for (channel, source) in colored.color.into_iter().zip(options.color) {
        close(channel, redstone.color[0] * source);
    }
}

#[test]
fn distance_rejection_precedes_constructor_randomness_for_every_request() {
    let mut store = empty_store();
    for create in registered_spawns() {
        fastrand::seed(18);
        let expected = fastrand::u64(..);
        fastrand::seed(18);
        create(&mut store.local_spawns(DVec3::splat(-1000.0)));
        assert_eq!(fastrand::u64(..), expected);
        assert_eq!(store.len(), 0);
    }
}

fn spawn<P: ParticleProvider + 'static>(
    context: &mut ParticleSpawnContext<'_>,
    options: P::Options,
    position: DVec3,
    velocity: DVec3,
) {
    context
        .system
        .get_generator::<P>()
        .unwrap()
        .create(context, options, position, velocity)
        .unwrap();
}

// A definition outside the built-in list, with its own creation options.
// Neither the engine nor the built-in assembly needs a new variant or decoder.
struct TestDefinition;
struct TestOptions {
    size: f32,
}

impl ParticleProvider for TestDefinition {
    type Options = TestOptions;
    type State = ParticleState;

    fn create(&self, options: TestOptions, position: DVec3, velocity: DVec3) -> ParticleState {
        let mut state = Smoke { frames: frames() }.create((), position, velocity);
        state.size = options.size;
        state.lifetime = 2;
        state.gravity = 0.0;
        state
    }
}

impl ParticleBehavior for TestDefinition {
    type State = ParticleState;

    fn tick(&self, state: &mut ParticleState, _chunks: &ChunkStore) -> bool {
        if !state.begin_tick() {
            return false;
        }
        state.pos += state.vel;
        true
    }

    fn extract(
        &self,
        state: &ParticleState,
        partial_tick: f32,
        anchor: DVec3,
        _chunks: &ChunkStore,
    ) -> ParticleQuad {
        state.quad(partial_tick, anchor, state.size, 1.0, false)
    }
}

#[test]
fn new_definition_registers_eager_typed_route_without_catalogue_changes() {
    let mut store = empty_store();
    let batch_count = store.batches.len();
    for _ in 0..2 {
        assert!(matches!(
            store.get_generator::<TestDefinition>(),
            Err(RegistrationError::MissingDefinition(_))
        ));
        assert_eq!(
            store.batches.len(),
            batch_count,
            "lookup must never register"
        );
    }
    store.register(TestDefinition).unwrap();
    let generator = store.get_generator::<TestDefinition>().unwrap().clone();
    assert_eq!(store.batches.len(), batch_count + 1);
    generator
        .create(
            &mut store.local_spawns(DVec3::ZERO),
            TestOptions { size: 0.375 },
            DVec3::ZERO,
            DVec3::X,
        )
        .unwrap();
    let chunks = ChunkStore::with_origin_chunk();
    assert_eq!(store.len(), 1);
    assert!(store.extract(0.0, DVec3::ZERO, &chunks).is_empty());
    store.tick(&chunks);
    let output = store.extract(1.0, DVec3::ZERO, &chunks);
    assert_eq!(output[0].pos, [0.0; 3]);
    assert_eq!(output[0].size, 0.375);
    store.tick(&chunks);
    assert_eq!(
        store.extract(0.5, DVec3::ZERO, &chunks)[0].pos,
        [0.5, 0.0, 0.0]
    );
    store.tick(&chunks);
    store.tick(&chunks);
    assert_eq!(store.len(), 0);
}

#[test]
fn wrong_system_and_reload_handles_reject_before_construction() {
    let mut original = empty_store();
    let handle = particle_defs::flame::get_generator(&original).unwrap();
    handle
        .create(
            &mut original.local_spawns(DVec3::ZERO),
            FlameFlavor::Copper,
            DVec3::ZERO,
            DVec3::ZERO,
        )
        .unwrap();
    let mut other = empty_store();
    for stale in [false, true] {
        if stale {
            original = empty_store();
        }
        let target = if stale { &mut original } else { &mut other };
        for position in [DVec3::ZERO, DVec3::splat(1000.0)] {
            fastrand::seed(56);
            let next = fastrand::u64(..);
            fastrand::seed(56);
            assert_eq!(
                handle.clone().create(
                    &mut target.local_spawns(DVec3::ZERO),
                    FlameFlavor::Copper,
                    position,
                    DVec3::ZERO
                ),
                Err(RegistrationError::WrongSystem)
            );
            assert_eq!(fastrand::u64(..), next);
            assert_eq!(target.len(), 0);
        }
        assert_eq!(
            handle.storage.borrow().pending.len(),
            1,
            "detached queue must not receive stale spawns"
        );
    }
    particle_defs::flame::get_generator(&original)
        .unwrap()
        .create(
            &mut original.local_spawns(DVec3::ZERO),
            FlameFlavor::Soul,
            DVec3::ZERO,
            DVec3::ZERO,
        )
        .unwrap();
    assert_eq!(original.len(), 1);
}

// A fresh binding distinguishes identity rejection from duplicate detection.
struct SharedProbe;

impl ParticleProvider for SharedProbe {
    type Options = (AtlasRegion, f32);
    type State = ParticleState;

    fn create(&self, options: Self::Options, position: DVec3, velocity: DVec3) -> Self::State {
        ItemDebris.create(options, position, velocity)
    }
}

fn assert_shared_registration_rejects_foreign_handle(stale: bool) {
    let mut source = empty_store();
    let handle = source.get_generator::<Terrain>().unwrap();
    spawn::<ItemDebris>(
        &mut source.source_spawns(),
        (frames()[0], 1.0),
        DVec3::ZERO,
        DVec3::ZERO,
    );
    let mut target = empty_store();
    let batches = target.batches.len();
    let bindings = target.bindings.len();
    if stale {
        source = empty_store();
        assert!(handle.system.upgrade().is_none());
    }
    let source_count = source.len();
    fastrand::seed(109);
    let next = fastrand::u64(..);
    fastrand::seed(109);
    assert_eq!(
        target.register_shared(SharedProbe, &handle),
        Err(RegistrationError::WrongSystem)
    );
    assert!(matches!(
        target.get_generator::<SharedProbe>(),
        Err(RegistrationError::MissingDefinition(_))
    ));
    assert_eq!(target.batches.len(), batches);
    assert_eq!(target.bindings.len(), bindings);
    assert_eq!(target.len(), 0);
    assert_eq!(source.len(), source_count);
    assert_eq!(handle.storage.borrow().pending.len(), 1);
    assert!(handle.storage.borrow().live.is_empty());
    assert_eq!(fastrand::u64(..), next);

    // Rejection leaves the type available for a valid local registration.
    let local = target.get_generator::<Terrain>().unwrap();
    assert!(!Rc::ptr_eq(&local.storage, &handle.storage));
    target.register_shared(SharedProbe, &local).unwrap();
    let shared = target.get_generator::<SharedProbe>().unwrap();
    assert!(Rc::ptr_eq(&shared.storage, &local.storage));
    shared
        .create(
            &mut target.source_spawns(),
            (frames()[1], 0.5),
            DVec3::X,
            DVec3::ZERO,
        )
        .unwrap();
    assert_eq!(target.len(), 1);
    assert_eq!(target.batches.len(), batches);
    assert_eq!(target.bindings.len(), bindings + 1);
    assert_eq!(source.len(), source_count);
    assert_eq!(handle.storage.borrow().pending.len(), 1);
    let chunks = ChunkStore::with_origin_chunk();
    target.tick(&chunks);
    assert_eq!(
        target.extract(0.0, DVec3::ZERO, &chunks)[0].pos,
        [1.0, 0.0, 0.0]
    );
}

#[test]
fn shared_registration_rejects_foreign_system() {
    assert_shared_registration_rejects_foreign_handle(false);
}

#[test]
fn shared_registration_rejects_reload_stale_system() {
    assert_shared_registration_rejects_foreign_handle(true);
}

#[test]
fn duplicate_registration_preserves_bindings_batches_and_queued_particles() {
    let mut store = empty_store();
    let terrain = store.get_generator::<Terrain>().unwrap();
    let item = store.get_generator::<ItemDebris>().unwrap();
    spawn::<ItemDebris>(
        &mut store.source_spawns(),
        (frames()[0], 1.0),
        DVec3::ZERO,
        DVec3::ZERO,
    );
    let batches = store.batches.len();
    let bindings = store.bindings.len();
    fastrand::seed(110);
    let next = fastrand::u64(..);
    fastrand::seed(110);
    assert!(matches!(
        store.register(Terrain),
        Err(RegistrationError::DuplicateDefinition(_))
    ));
    assert!(matches!(
        store.register_shared(ItemDebris, &terrain),
        Err(RegistrationError::DuplicateDefinition(_))
    ));
    assert_eq!(fastrand::u64(..), next);
    assert_eq!(store.batches.len(), batches);
    assert_eq!(store.bindings.len(), bindings);
    assert_eq!(store.len(), 1);
    let current_terrain = store.get_generator::<Terrain>().unwrap();
    let current_item = store.get_generator::<ItemDebris>().unwrap();
    assert!(Rc::ptr_eq(&terrain.storage, &current_terrain.storage));
    assert!(Rc::ptr_eq(&terrain.provider, &current_terrain.provider));
    assert!(Rc::ptr_eq(&item.storage, &current_item.storage));
    assert!(Rc::ptr_eq(&item.provider, &current_item.provider));
    let chunks = ChunkStore::with_origin_chunk();
    store.tick(&chunks);
    assert_eq!(store.extract(0.0, DVec3::ZERO, &chunks).len(), 1);
}

#[test]
fn terrain_and_item_generators_share_one_typed_engine_batch() {
    let mut store = empty_store();
    let terrain = particle_defs::terrain::get_generator(&store).unwrap();
    let item = particle_defs::terrain::get_item_generator(&store).unwrap();
    assert!(Rc::ptr_eq(&terrain.storage, &item.storage));
    terrain
        .create(
            &mut store.source_spawns(),
            TerrainOptions {
                region: frames()[2],
                color: [0.6; 3],
                light: 0.5,
            },
            DVec3::ZERO,
            DVec3::ZERO,
        )
        .unwrap();
    item.create(
        &mut store.source_spawns(),
        (frames()[5], 0.75),
        DVec3::X,
        DVec3::ZERO,
    )
    .unwrap();
    assert_eq!(store.len(), 2);
    assert_eq!(terrain.storage.borrow().pending.len(), 2);
    let chunks = ChunkStore::with_origin_chunk();
    store.tick(&chunks);
    let output = store.extract(0.0, DVec3::ZERO, &chunks);
    assert_eq!(output.len(), 2);
    assert_eq!(output[0].pos, [0.0; 3]);
    assert_eq!(output[1].pos, [1.0, 0.0, 0.0]);
    store.tick(&chunks);
    assert!(
        terrain
            .storage
            .borrow()
            .live
            .iter()
            .all(|state| state.age == 1)
    );
}

/// Independent typed sources, not an options envelope or erased state capture.
fn registered_spawns() -> [fn(&mut ParticleSpawnContext<'_>); 8] {
    const POS: DVec3 = DVec3::new(0.5, 80.0, 0.5);
    const VEL: DVec3 = DVec3::new(0.1, 0.2, 0.3);
    [
        |s| spawn::<Smoke>(s, (), POS, VEL),
        |s| spawn::<Flame>(s, FlameFlavor::Ordinary, POS, VEL),
        |s| spawn::<Flame>(s, FlameFlavor::Soul, POS, VEL),
        |s| spawn::<Flame>(s, FlameFlavor::Copper, POS, VEL),
        |s| spawn::<Dust>(s, DustOptions::redstone(), POS, VEL),
        |s| {
            spawn::<Terrain>(
                s,
                TerrainOptions {
                    region: frames()[0],
                    color: [0.6; 3],
                    light: 1.0,
                },
                POS,
                VEL,
            )
        },
        |s| spawn::<ItemDebris>(s, (frames()[0], 1.0), POS, VEL),
        |s| spawn::<EndRod>(s, (), POS, VEL),
    ]
}

#[test]
fn every_registered_batch_activates_without_ticking_new_particles() {
    let chunks = ChunkStore::with_origin_chunk();
    let sources = registered_spawns();
    // Isolate each route; only within-family insertion order is contractual.
    // The mixed-batch barrier and global cap have separate tests.
    for create in sources {
        let mut store = empty_store();
        for round in 0..2 {
            create(&mut store.source_spawns());
            assert_eq!(store.len(), round + 1);
            assert_eq!(store.extract(0.0, DVec3::ZERO, &chunks).len(), round);
            store.tick(&chunks);
            let output = store.extract(0.0, DVec3::ZERO, &chunks);
            assert_eq!(output.len(), round + 1);
            assert_eq!(output.last().unwrap().pos, [0.5, 80.0, 0.5]);
        }
        for _ in 0..100 {
            store.tick(&chunks);
        }
        assert_eq!(store.len(), 0);
    }
    let mut store = empty_store();
    for create in sources {
        create(&mut store.source_spawns());
    }
    assert!(store.extract(0.0, DVec3::ZERO, &chunks).is_empty());
    store.tick(&chunks);
    assert_eq!(
        store.extract(0.0, DVec3::ZERO, &chunks).len(),
        sources.len()
    );
}

#[test]
fn all_live_ticks_precede_any_pending_activation() {
    struct ObservedBatch {
        id: usize,
        events: Rc<RefCell<Vec<(usize, bool)>>>,
    }
    impl ParticleBatchHandler for ObservedBatch {
        fn tick_live(&mut self, _chunks: &ChunkStore) {
            self.events.borrow_mut().push((self.id, false));
        }
        fn activate_pending(&mut self) {
            self.events.borrow_mut().push((self.id, true));
        }
        fn extract(
            &self,
            _partial_tick: f32,
            _anchor: DVec3,
            _chunks: &ChunkStore,
            _output: &mut Vec<ParticleQuad>,
        ) {
        }
        fn len(&self) -> usize {
            0
        }
    }
    let mut store = empty_store();
    let events = Rc::new(RefCell::new(Vec::new()));
    let batch_count = 3;
    for id in 0..batch_count {
        store.batches.push(Box::new(ObservedBatch {
            id,
            events: Rc::clone(&events),
        }));
    }
    store.tick(&ChunkStore::with_origin_chunk());
    let events = events.borrow();
    assert_eq!(events.len(), batch_count * 2);
    for id in 0..batch_count {
        for activation in [false, true] {
            assert_eq!(
                events
                    .iter()
                    .filter(|&&event| event == (id, activation))
                    .count(),
                1,
                "batch {id} must receive exactly one event with activation={activation}"
            );
        }
    }
    let mut activation_started = false;
    for &(_, activation) in events.iter() {
        if activation {
            activation_started = true;
        } else {
            assert!(!activation_started, "live tick occurred after activation");
        }
    }
}

/// Test-only typed queue setup avoids improbable reservoir draws. No engine
/// state recovery or production admission bypass is exposed.
fn fill_mixed(store: &mut ParticleStore, count: usize) {
    let smoke = store.get_generator::<Smoke>().unwrap();
    let end_rod = store.get_generator::<EndRod>().unwrap();
    for i in 0..count {
        if i % 2 == 0 {
            smoke
                .storage
                .borrow_mut()
                .pending
                .push(smoke.provider.create((), DVec3::ZERO, DVec3::ZERO));
        } else {
            end_rod
                .storage
                .borrow_mut()
                .pending
                .push(end_rod.provider.create((), DVec3::ZERO, DVec3::ZERO));
        }
        if i == count / 2 {
            store.tick(&ChunkStore::with_origin_chunk());
        }
    }
}

#[test]
fn caps_and_reservoir_count_all_live_and_pending_after_construction() {
    let chunks = ChunkStore::with_origin_chunk();
    let count = RESERVOIR_START + 2048;
    // Compare the reservoir outcome after construction, not a private draw count.
    let mut accepted = 0;
    for seed in 0..16 {
        let mut store = empty_store();
        fill_mixed(&mut store, count);
        let live = store.extract(0.0, DVec3::ZERO, &chunks).len();
        assert!(live > 0 && live < count);
        fastrand::seed(seed);
        let _ = Smoke { frames: frames() }.create((), DVec3::ZERO, DVec3::ZERO);
        let expected = fastrand::f32() < 0.25;
        fastrand::seed(seed);
        spawn::<Smoke>(
            &mut store.local_spawns(DVec3::ZERO),
            (),
            DVec3::ZERO,
            DVec3::ZERO,
        );
        assert_eq!(store.len(), count + usize::from(expected));
        if expected {
            accepted += 1;
        }
    }
    assert!((1..16).contains(&accepted));
    let mut store = empty_store();
    fill_mixed(&mut store, MAX_PARTICLES);
    assert_eq!(store.len(), MAX_PARTICLES);
    for create in registered_spawns() {
        let mut reference = empty_store();
        fastrand::seed(91);
        create(&mut reference.source_spawns());
        let next = fastrand::u64(..);
        fastrand::seed(91);
        create(&mut store.source_spawns());
        assert_eq!(fastrand::u64(..), next);
        assert_eq!(store.len(), MAX_PARTICLES);
    }
    fastrand::seed(91);
    let _ = EndRod { frames: frames() }.create((), DVec3::X * 100.0, DVec3::Y);
    let expected_next = fastrand::u64(..);
    fastrand::seed(91);
    store.add_particles_from_packet(
        ServerParticleKind::EndRod,
        true,
        DVec3::X * 100.0,
        DVec3::Y,
        1.0,
        0,
        DVec3::ZERO,
    );
    assert_eq!(fastrand::u64(..), expected_next);
    store.add_item_use_particles(1, FLAME_SPRITES[0], DVec3::ZERO, 0.0, 0.0, &chunks);
    spawn::<Dust>(
        &mut store.local_spawns(DVec3::ZERO),
        DustOptions::redstone(),
        DVec3::ZERO,
        DVec3::ZERO,
    );
    assert_eq!(store.len(), MAX_PARTICLES);
}

#[test]
fn server_direction_scatter_distance_override_and_end_rod_order() {
    assert!(matches!(
        ServerParticleKind::from_id(27),
        Some(ServerParticleKind::EndRod)
    ));
    assert!(ServerParticleKind::from_id(26).is_none());
    assert!(ServerParticleKind::from_id(28).is_none());
    let chunks = ChunkStore::with_origin_chunk();
    let mut store = empty_store();
    let pos = dvec3(32.0, 0.0, 0.0);
    let dist = dvec3(0.2, 0.3, 0.4);
    store.add_particles_from_packet(
        ServerParticleKind::EndRod,
        false,
        pos,
        dist,
        2.0,
        0,
        DVec3::ZERO,
    );
    store.add_particles_from_packet(
        ServerParticleKind::EndRod,
        false,
        pos + DVec3::X,
        dist,
        2.0,
        0,
        DVec3::ZERO,
    );
    assert_eq!(store.len(), 1);
    store.add_particles_from_packet(
        ServerParticleKind::EndRod,
        true,
        pos + DVec3::X,
        dist,
        2.0,
        0,
        DVec3::ZERO,
    );
    assert_eq!(store.len(), 2);
    // Choose the camera from the actual scatter so the packet center is far away.
    fastrand::seed(28);
    let scatter = dvec3(next_gaussian(), next_gaussian(), next_gaussian()) * 100.0;
    let expected_vel = dvec3(next_gaussian(), next_gaussian(), next_gaussian()) * 2.0;
    assert!(scatter.length_squared() > 1024.0);
    fastrand::seed(28);
    store.add_particles_from_packet(
        ServerParticleKind::EndRod,
        false,
        pos,
        DVec3::splat(100.0),
        2.0,
        1,
        pos + scatter,
    );
    assert_eq!(store.len(), 3);
    let initial = [pos, pos + DVec3::X, pos + scatter];
    spawn::<Smoke>(&mut store.local_spawns(pos), (), pos, DVec3::ZERO);
    store.tick(&chunks);
    let quads = store.extract(0.0, DVec3::ZERO, &chunks);
    assert_eq!(
        quads
            .iter()
            .filter(|q| q.translucent)
            .map(|q| q.pos)
            .collect::<Vec<_>>(),
        initial.map(|p| p.as_vec3().to_array()).to_vec()
    );
    store.tick(&chunks);
    let moved = store.extract(1.0, DVec3::ZERO, &chunks);
    for (quad, (position, velocity)) in
        moved
            .iter()
            .filter(|q| q.translucent)
            .zip(
                initial
                    .into_iter()
                    .zip([dist * 2.0, dist * 2.0, expected_vel]),
            )
    {
        let expected = position + velocity - DVec3::Y * (0.04 * f64::from(0.0125f32));
        assert_eq!(quad.pos, expected.as_vec3().to_array());
    }
}

#[test]
fn item_use_particles_bypass_local_culling() {
    let chunks = ChunkStore::with_origin_chunk();
    let mut store = empty_store();
    store.add_item_use_particles(2, FLAME_SPRITES[0], DVec3::splat(1000.0), 0.0, 0.0, &chunks);
    assert_eq!(store.len(), 2);
    assert!(store.extract(0.0, DVec3::ZERO, &chunks).is_empty());
    store.tick(&chunks);
    assert!(
        store
            .extract(0.0, DVec3::ZERO, &chunks)
            .iter()
            .all(|q| !q.translucent && q.pos[0] > 999.0)
    );
}

#[test]
fn rebuilding_store_clears_live_and_pending_and_rebinds_sprites() {
    let chunks = ChunkStore::with_origin_chunk();
    let mut store = empty_store();
    fill_mixed(&mut store, 10);
    assert_eq!(store.len(), 10);
    let region = frames()[6];
    store = ParticleStore::new(
        AtlasUVMap::with_regions(region, sprite_names().map(|name| (name.to_owned(), region))),
        Arc::clone(&store.grass_colormap),
        Arc::clone(&store.foliage_colormap),
        Arc::clone(&store.dry_foliage_colormap),
    )
    .unwrap();
    assert_eq!(store.len(), 0);
    store.tick(&chunks);
    assert!(store.extract(0.0, DVec3::ZERO, &chunks).is_empty());
    for create in registered_spawns()
        .into_iter()
        .take(5)
        .chain(registered_spawns().into_iter().skip(7))
    {
        create(&mut store.source_spawns());
    }
    store.tick(&chunks);
    assert!(store.extract(0.0, DVec3::ZERO, &chunks).iter().all(|q| {
        (q.u0, q.u1, q.v0, q.v1) == (region.u_min, region.u_max, region.v_min, region.v_max)
    }));
}
