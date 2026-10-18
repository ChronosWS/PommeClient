//! Compile the extension contract from a sibling, without particle internals.

use glam::DVec3;

use crate::particle::tests::empty_store;
use crate::particle::{ParticleBehavior, ParticleEmitter, ParticleProvider, RegistrationError};
use crate::renderer::ParticleQuad;
use crate::world::chunk::ChunkStore;

struct Definition;

struct Options {
    size: f32,
    lifetime: u32,
}

struct State {
    position: DVec3,
    previous: DVec3,
    velocity: DVec3,
    age: u32,
    options: Options,
}

impl ParticleProvider for Definition {
    type Options = Options;
    type State = State;

    fn create(&self, options: Options, position: DVec3, velocity: DVec3) -> State {
        State {
            position,
            previous: position,
            velocity,
            age: 0,
            options,
        }
    }
}

impl ParticleBehavior for Definition {
    type State = State;

    fn tick(&self, state: &mut State, _chunks: &ChunkStore) -> bool {
        if state.age >= state.options.lifetime {
            return false;
        }
        state.age += 1;
        state.previous = state.position;
        state.position += state.velocity;
        true
    }

    fn extract(
        &self,
        state: &State,
        partial_tick: f32,
        anchor: DVec3,
        _chunks: &ChunkStore,
    ) -> ParticleQuad {
        ParticleQuad {
            pos: (state.previous.lerp(state.position, f64::from(partial_tick)) - anchor)
                .as_vec3()
                .to_array(),
            size: state.options.size,
            u0: 0.0,
            u1: 1.0,
            v0: 0.0,
            v1: 1.0,
            color: u32::MAX,
            translucent: false,
        }
    }
}

#[test]
fn sibling_definition_registers_and_uses_its_own_options_and_state() {
    let mut store = empty_store();
    for _ in 0..2 {
        assert!(matches!(
            store.get_generator::<Definition>(),
            Err(RegistrationError::MissingDefinition(_))
        ));
    }
    let registered: ParticleEmitter<Definition> = store.register(Definition).unwrap();
    let acquired: ParticleEmitter<Definition> =
        store.get_generator::<Definition>().unwrap().clone();
    for (generator, position) in [(registered, DVec3::ZERO), (acquired, DVec3::Y)] {
        generator
            .create(
                &mut store.local_spawns(DVec3::ZERO),
                Options {
                    size: 0.375,
                    lifetime: 2,
                },
                position,
                DVec3::X,
            )
            .unwrap();
    }
    let chunks = ChunkStore::with_origin_chunk();
    assert!(store.extract(1.0, DVec3::ZERO, &chunks).is_empty());
    store.tick(&chunks);
    let output = store.extract(1.0, DVec3::ZERO, &chunks);
    assert_eq!(output.len(), 2);
    assert_eq!(output[0].pos, [0.0, 0.0, 0.0]);
    assert_eq!(output[1].pos, [0.0, 1.0, 0.0]);
    assert!(output.iter().all(|q| q.size == 0.375));
    store.tick(&chunks);
    let output = store.extract(0.5, DVec3::ZERO, &chunks);
    assert_eq!(output[0].pos, [0.5, 0.0, 0.0]);
    assert_eq!(output[1].pos, [0.5, 1.0, 0.0]);
    store.tick(&chunks);
    assert_eq!(store.extract(1.0, DVec3::ZERO, &chunks).len(), 2);
    store.tick(&chunks);
    assert!(store.extract(1.0, DVec3::ZERO, &chunks).is_empty());
}
