//! Definition-owned typed emitters and batches; erased runtime operations
//! only.
//!
//! After resources load, fallible built-in initializers register typed emitter
//! bindings and their shared batch storage. A definition getter recovers an
//! owned handle by definition identity. `ParticleEmitter::create` validates its
//! scoped spawn service, culls distance, constructs typed state, then checks
//! global capacity and queues. The engine ticks/extracts typed vectors through
//! one erased adapter per batch.
//!
//! # Adding a particle emitter
//!
//! Add a `particle_defs` module implementing `ParticleProvider` with typed
//! `Options`, `State`, and `create`. For a new batch, implement
//! `ParticleBehavior` (`tick`/`extract`) and use `ParticleStore::register`.
//! Or use `ParticleStore::register_shared` with an emitter of the same `State`,
//! without implementing behavior (as `terrain::ItemDebris` does). The
//! implementor chooses the compatible behavior semantics.
//! Own sprite names and atlas-bound construction in the definition; add new
//! required sprites to `particle_defs::sprite_names` for loading and reload.
//! Add an eager `initialize` to the list in `particle_defs::initialize`.
//! Expose a typed `get_generator` wrapping `ParticleStore::get_generator`;
//! getters only retrieve existing bindings, never initialize lazily.
//! Acquire handles at producer construction when possible; reacquire when
//! replacing the store. Call `ParticleEmitter::create` with
//! `ParticleSpawnContext`, typed options, position, and velocity. No
//! generic-store enum changes are needed. Checked `Any` recovery is only for
//! bindings, never options or state.
//!
//! # Rendering flow
//!
//! The store extracts live batch state at the partial tick into
//! camera-anchor-relative `ParticleQuad`s. Using each quad's `translucent`
//! flag, the renderer builds opaque/cutout vertices first, then alpha-blended
//! translucent vertices, and draws each layer with its corresponding pipeline.
//! Input need not be grouped by layer; within-layer order is preserved, with no
//! depth sorting.
//!
//! # Differences from Vanilla 26.3
//!
//! - **Storage and dispatch** (`batch`): each behavior-family batch owns typed,
//!   plain-state live/pending vectors and shared providers/behavior, with one
//!   erased handler per batch instead of per-instance Java polymorphic objects.
//! - **Identity and resources** (`particle_defs`): provider types identify
//!   bindings, not Vanilla registry or native type IDs. Built-in initialization
//!   is explicit and compiled, unlike Vanilla's registry and
//!   `ParticleResources`. Sprite lists are hardcoded in family definitions and
//!   use a shared atlas, rather than resource JSON feeding mutable
//!   `SpriteSet`s. `ParticleStore::new` propagates registration errors, never
//!   returning a partial store. `LevelRuntime` owns the store and its paired
//!   block definitions; reload builds new definitions before replacing both,
//!   retaining the old pair on failure and preserving coordinate sampling RNG.
//!   Reload replaces the entire store, clearing live/pending particles, rather
//!   than rebinding sprite sets; Vanilla also clears particles on reload.
//! - **Lighting** (`state`, `particle_defs/flame`): CPU scalar brightness uses
//!   max(sky, block) with an emission approximation for flame. It lacks
//!   Vanilla's full RGB/dynamic lightmap and time-of-day skylight adjustment,
//!   so nighttime particle lighting is a known deficiency.
//! - **Rendering** (`renderer::pipelines::particle`): currently only opaque and
//!   translucent shared-atlas billboards are supported, not Vanilla's
//!   additional atlases, special render groups, or optional order-independent
//!   transparency. Neither ordinary quad path depth-sorts; no new transparency
//!   feature is intended here.
//! - **Admission** (`admit`, `batch`): one global limit counts live + pending,
//!   rejecting after construction but before enqueue. Vanilla's normal quad
//!   groups check admission during pending drain, so equal thresholds do not
//!   imply identical burst behavior.
//! - **Randomness and precision** (`world::level`, `particle_defs`): sampling
//!   and effect streams are independent; matching distributions does not imply
//!   seeded sequence equivalence. Constructors and admission use thread-local
//!   `fastrand`, not Vanilla's per-particle/engine RNG ownership. Smoke/flame
//!   lifetime arithmetic intentionally diverges using `f32` (the flame
//!   reference uses double precision); dust retains `f64` lifetime arithmetic.
//! - **Source adapters** (`effects`): block/item effect adapters remain methods
//!   on `ParticleStore`, rather than being spread across Vanilla `ClientLevel`
//!   routines.

mod batch;
mod effects;
pub(crate) mod particle_defs;
mod state;

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use batch::{BatchStorage, ParticleBatch, ParticleBatchHandler};
pub(crate) use batch::{ParticleBehavior, ParticleEmitter, ParticleProvider};
pub use effects::ServerParticleKind;
use glam::DVec3;
pub use particle_defs::sprite_names;

use crate::renderer::ParticleQuad;
use crate::renderer::chunk::atlas::AtlasUVMap;
use crate::renderer::chunk::mesher::Colormap;
use crate::renderer::pipelines::particle::MAX_PARTICLE_QUADS as MAX_PARTICLES;
use crate::world::chunk::ChunkStore;

/// Vanilla `ParticleGroup.RESERVOIR_START`.
const RESERVOIR_START: usize = 12288;

pub struct ParticleStore {
    batches: Vec<Box<dyn ParticleBatchHandler>>,
    bindings: HashMap<TypeId, Box<dyn Any>>,
    identity: Rc<()>,
    uv_map: AtlasUVMap,
    grass_colormap: Arc<Colormap>,
    foliage_colormap: Arc<Colormap>,
    dry_foliage_colormap: Arc<Colormap>,
}

impl ParticleStore {
    pub fn new(
        uv_map: AtlasUVMap,
        grass_colormap: Arc<Colormap>,
        foliage_colormap: Arc<Colormap>,
        dry_foliage_colormap: Arc<Colormap>,
    ) -> Result<Self, RegistrationError> {
        Self::initialize(
            uv_map,
            grass_colormap,
            foliage_colormap,
            dry_foliage_colormap,
            particle_defs::initialize,
        )
    }

    fn initialize(
        uv_map: AtlasUVMap,
        grass_colormap: Arc<Colormap>,
        foliage_colormap: Arc<Colormap>,
        dry_foliage_colormap: Arc<Colormap>,
        initialize: impl FnOnce(&mut Self) -> Result<(), RegistrationError>,
    ) -> Result<Self, RegistrationError> {
        let mut store = Self {
            batches: Vec::new(),
            bindings: HashMap::new(),
            identity: Rc::new(()),
            uv_map,
            grass_colormap,
            foliage_colormap,
            dry_foliage_colormap,
        };
        initialize(&mut store)?;
        Ok(store)
    }

    fn len(&self) -> usize {
        self.batches.iter().map(|batch| batch.len()).sum()
    }

    pub(crate) fn local_spawns(&mut self, camera_pos: DVec3) -> ParticleSpawnContext<'_> {
        ParticleSpawnContext {
            system: self,
            camera: Some(camera_pos),
        }
    }

    fn source_spawns(&mut self) -> ParticleSpawnContext<'_> {
        ParticleSpawnContext {
            system: self,
            camera: None,
        }
    }

    /// Definition identity is the provider type, not its (possibly unit)
    /// options.
    pub(crate) fn get_generator<P: ParticleProvider + 'static>(
        &self,
    ) -> Result<ParticleEmitter<P>, RegistrationError> {
        self.bindings
            .get(&TypeId::of::<P>())
            .and_then(|binding| binding.downcast_ref::<ParticleEmitter<P>>())
            .cloned()
            .ok_or(RegistrationError::MissingDefinition(
                std::any::type_name::<P>(),
            ))
    }

    /// Eagerly bind a definition to a new batch; replacement is not supported.
    pub(crate) fn register<P>(
        &mut self,
        provider: P,
    ) -> Result<ParticleEmitter<P>, RegistrationError>
    where
        P: ParticleProvider + ParticleBehavior<State = <P as ParticleProvider>::State> + 'static,
    {
        self.check_unregistered::<P>()?;
        let provider = Rc::new(provider);
        let storage = Rc::new(RefCell::new(BatchStorage::default()));
        self.batches.push(Box::new(ParticleBatch {
            behavior: Rc::clone(&provider),
            storage: Rc::clone(&storage),
        }));
        let generator = ParticleEmitter {
            provider,
            storage,
            system: Rc::downgrade(&self.identity),
        };
        self.bindings
            .insert(TypeId::of::<P>(), Box::new(generator.clone()));
        Ok(generator)
    }

    /// Bind a compatible provider to an existing batch owned by this store.
    /// The same state type is required; the implementor chooses behavior
    /// semantics.
    pub(crate) fn register_shared<P, B>(
        &mut self,
        provider: P,
        batch: &ParticleEmitter<B>,
    ) -> Result<(), RegistrationError>
    where
        P: ParticleProvider<State = B::State> + 'static,
        B: ParticleProvider,
    {
        // The original weak identity proves ownership, including when a handle
        // survives a resource reload.
        if !batch.system.ptr_eq(&Rc::downgrade(&self.identity)) {
            return Err(RegistrationError::WrongSystem);
        }
        self.check_unregistered::<P>()?;
        let generator = ParticleEmitter {
            provider: Rc::new(provider),
            storage: Rc::clone(&batch.storage),
            system: Rc::downgrade(&self.identity),
        };
        self.bindings.insert(TypeId::of::<P>(), Box::new(generator));
        Ok(())
    }

    fn check_unregistered<P: 'static>(&self) -> Result<(), RegistrationError> {
        if self.bindings.contains_key(&TypeId::of::<P>()) {
            return Err(RegistrationError::DuplicateDefinition(
                std::any::type_name::<P>(),
            ));
        }
        Ok(())
    }

    pub fn tick(&mut self, chunks: &ChunkStore) {
        for batch in &mut self.batches {
            batch.tick_live(chunks);
        }
        // Global barrier: no newly queued particle simulates on its first tick.
        for batch in &mut self.batches {
            batch.activate_pending();
        }
    }

    pub fn extract(
        &self,
        partial_tick: f32,
        anchor: DVec3,
        chunks: &ChunkStore,
    ) -> Vec<ParticleQuad> {
        let mut output = Vec::with_capacity(self.len());
        for batch in &self.batches {
            batch.extract(partial_tick, anchor, chunks, &mut output);
        }
        output
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RegistrationError {
    MissingDefinition(&'static str),
    DuplicateDefinition(&'static str),
    WrongSystem,
}

/// Scoped common creation service. Only source adapters may bypass distance.
pub(crate) struct ParticleSpawnContext<'a> {
    system: &'a mut ParticleStore,
    camera: Option<DVec3>,
}

impl ParticleSpawnContext<'_> {
    fn admission(
        &self,
        identity: &Weak<()>,
        position: DVec3,
    ) -> Result<Option<Admission>, RegistrationError> {
        if !identity.ptr_eq(&Rc::downgrade(&self.system.identity)) {
            return Err(RegistrationError::WrongSystem);
        }
        if self
            .camera
            .is_some_and(|camera| camera.distance_squared(position) > 1024.0)
        {
            return Ok(None);
        }
        // Count before borrowing the selected typed storage, avoiding reentrant RefCell
        // borrows.
        Ok(Some(Admission {
            count: self.system.len(),
        }))
    }
}

struct Admission {
    count: usize,
}

impl Admission {
    fn accepts(self) -> bool {
        admit(self.count)
    }
}

/// All sources check live + pending across every batch, after construction.
fn admit(count: usize) -> bool {
    if count >= MAX_PARTICLES {
        return false;
    }
    if count >= RESERVOIR_START {
        let free = (MAX_PARTICLES - count) as f32 / 4096.0;
        if fastrand::f32() >= free * free {
            return false;
        }
    }
    true
}

/// `java.util.Random.nextGaussian` (Marsaglia polar method), minus the cache.
pub(crate) fn next_gaussian() -> f64 {
    loop {
        let v1 = 2.0 * fastrand::f64() - 1.0;
        let v2 = 2.0 * fastrand::f64() - 1.0;
        let s = v1 * v1 + v2 * v2;
        if s < 1.0 && s != 0.0 {
            return v1 * (-2.0 * s.ln() / s).sqrt();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
