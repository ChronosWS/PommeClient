use glam::DVec3;

use crate::physics::aabb::Aabb;
use crate::physics::block_shape::CollisionContext;
use crate::physics::collision::resolve_collision;
use crate::renderer::ParticleQuad;
use crate::renderer::chunk::atlas::AtlasRegion;
use crate::renderer::chunk::mesher::world_brightness;
use crate::world::chunk::ChunkStore;

/// Common instance data, with no behavior tag or owned sprite resources.
pub(crate) struct ParticleState {
    /// Bounding-box bottom-center, like vanilla `Particle.setPos`.
    pub pos: DVec3,
    pub prev_pos: DVec3,
    pub vel: DVec3,
    pub age: i32,
    pub lifetime: i32,
    pub on_ground: bool,
    pub stopped_by_collision: bool,
    pub gravity: f64,
    pub friction: f64,
    /// Vanilla `quadSize`; the billboard spans twice this.
    pub size: f32,
    pub u0: f32,
    pub u1: f32,
    pub v0: f32,
    pub v1: f32,
    pub color: [f32; 3],
    pub alpha: f32,
    pub light: f32,
}

impl ParticleState {
    pub fn set_sprite(&mut self, frame: &AtlasRegion) {
        self.u0 = frame.u_min;
        self.u1 = frame.u_max;
        self.v0 = frame.v_min;
        self.v1 = frame.v_max;
    }

    /// Preserve the age++ expiry boundary and previous-position update.
    pub fn begin_tick(&mut self) -> bool {
        self.prev_pos = self.pos;
        if self.age >= self.lifetime {
            return false;
        }
        self.age += 1;
        self.vel.y -= 0.04 * self.gravity;
        true
    }

    pub fn apply_drag(&mut self) {
        self.vel *= self.friction;
        if self.on_ground {
            self.vel.x *= 0.7;
            self.vel.z *= 0.7;
        }
    }

    pub fn world_light(&self, chunks: &ChunkStore) -> f32 {
        world_brightness(
            chunks,
            self.pos.x.floor() as i32,
            self.pos.y.floor() as i32,
            self.pos.z.floor() as i32,
        )
    }

    pub fn quad(
        &self,
        partial_tick: f32,
        anchor: DVec3,
        size: f32,
        light: f32,
        translucent: bool,
    ) -> ParticleQuad {
        // Interpolate and subtract the camera anchor before narrowing to f32.
        let pos = (self.prev_pos.lerp(self.pos, partial_tick as f64) - anchor).as_vec3();
        let channel = |c: f32| (c * light * 255.0).round() as u8;
        ParticleQuad {
            pos: pos.into(),
            size,
            u0: self.u0,
            u1: self.u1,
            v0: self.v0,
            v1: self.v1,
            color: u32::from_le_bytes([
                channel(self.color[0]),
                channel(self.color[1]),
                channel(self.color[2]),
                (self.alpha * 255.0).round() as u8,
            ]),
            translucent,
        }
    }

    /// Vanilla `Particle.move`, default 0.2-wide, 0.2-tall bounding box.
    pub fn move_with_collision(&mut self, chunks: &ChunkStore) {
        if self.stopped_by_collision {
            return;
        }
        let orig = self.vel;
        let mut delta = orig;
        if delta != DVec3::ZERO && delta.length_squared() < 10000.0 {
            let aabb = Aabb::from_center(self.pos, 0.1, 0.1);
            (delta, _) = resolve_collision(
                chunks.into(),
                aabb,
                orig.into(),
                0.0,
                false,
                &CollisionContext::position(self.pos.y),
            );
        }
        self.pos += delta;
        if orig.y.abs() >= 1e-5 && delta.y.abs() < 1e-5 {
            self.stopped_by_collision = true;
        }
        self.on_ground = orig.y != delta.y && orig.y < 0.0;
        if orig.x != delta.x {
            self.vel.x = 0.0;
        }
        if orig.z != delta.z {
            self.vel.z = 0.0;
        }
    }
}
