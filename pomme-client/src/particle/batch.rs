use std::cell::RefCell;
use std::rc::{Rc, Weak};

use glam::DVec3;

use super::{ParticleSpawnContext, RegistrationError};
use crate::renderer::ParticleQuad;
use crate::world::chunk::ChunkStore;

pub(crate) trait ParticleProvider {
    type Options;
    type State;

    fn create(&self, options: Self::Options, position: DVec3, velocity: DVec3) -> Self::State;
}

pub(crate) trait ParticleBehavior {
    type State;

    fn tick(&self, state: &mut Self::State, chunks: &ChunkStore) -> bool;
    fn extract(
        &self,
        state: &Self::State,
        partial_tick: f32,
        anchor: DVec3,
        chunks: &ChunkStore,
    ) -> ParticleQuad;
}

/// Runtime dispatch knows only common batch operations, never creation options.
pub(super) trait ParticleBatchHandler {
    fn tick_live(&mut self, chunks: &ChunkStore);
    fn activate_pending(&mut self);
    fn extract(
        &self,
        partial_tick: f32,
        anchor: DVec3,
        chunks: &ChunkStore,
        output: &mut Vec<ParticleQuad>,
    );
    fn len(&self) -> usize;
}

pub(super) struct BatchStorage<S> {
    pub live: Vec<S>,
    pub pending: Vec<S>,
}

impl<S> Default for BatchStorage<S> {
    fn default() -> Self {
        Self {
            live: Vec::new(),
            pending: Vec::new(),
        }
    }
}

pub(super) struct ParticleBatch<B: ParticleBehavior> {
    pub behavior: Rc<B>,
    pub storage: Rc<RefCell<BatchStorage<B::State>>>,
}

/// An owned, system-specific typed route. Cloning allocates no particle state.
pub(crate) struct ParticleEmitter<P: ParticleProvider> {
    pub(super) provider: Rc<P>,
    pub(super) storage: Rc<RefCell<BatchStorage<P::State>>>,
    pub(super) system: Weak<()>,
}

impl<P: ParticleProvider> Clone for ParticleEmitter<P> {
    fn clone(&self) -> Self {
        Self {
            provider: Rc::clone(&self.provider),
            storage: Rc::clone(&self.storage),
            system: self.system.clone(),
        }
    }
}

impl<P: ParticleProvider> ParticleEmitter<P> {
    pub fn create(
        &self,
        context: &mut ParticleSpawnContext<'_>,
        options: P::Options,
        position: DVec3,
        velocity: DVec3,
    ) -> Result<(), RegistrationError> {
        let Some(admission) = context.admission(&self.system, position)? else {
            return Ok(());
        };
        // Construction precedes capacity/reservoir admission, but follows culling.
        let state = self.provider.create(options, position, velocity);
        if admission.accepts() {
            self.storage.borrow_mut().pending.push(state);
        }
        Ok(())
    }
}

impl<B: ParticleBehavior> ParticleBatchHandler for ParticleBatch<B> {
    fn len(&self) -> usize {
        let storage = self.storage.borrow();
        storage.live.len() + storage.pending.len()
    }

    fn tick_live(&mut self, chunks: &ChunkStore) {
        self.storage
            .borrow_mut()
            .live
            .retain_mut(|state| self.behavior.tick(state, chunks));
    }

    fn activate_pending(&mut self) {
        let mut storage = self.storage.borrow_mut();
        let BatchStorage { live, pending } = &mut *storage;
        live.append(pending);
    }

    fn extract(
        &self,
        partial_tick: f32,
        anchor: DVec3,
        chunks: &ChunkStore,
        output: &mut Vec<ParticleQuad>,
    ) {
        output.extend(
            self.storage
                .borrow()
                .live
                .iter()
                .map(|state| self.behavior.extract(state, partial_tick, anchor, chunks)),
        );
    }
}
