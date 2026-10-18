//! Source-specific request generation; admission and simulation stay in the
//! store.

use std::collections::HashMap;

use azalea_block::BlockState;
use azalea_core::position::BlockPos;
use glam::{DVec3, dvec3};

use super::particle_defs::end_rod;
use super::particle_defs::terrain::{self, TerrainOptions};
use super::{ParticleStore, next_gaussian};
use crate::physics::block_shape::{self, LocalBox};
use crate::renderer::chunk::mesher::{
    BiomeClimate, blend_color, dry_foliage_color, foliage_color, grass_color, tint_sample_y,
    world_brightness,
};
use crate::world::block::registry::{BlockRegistry, Tint};
use crate::world::block::{block_id, is_air, outline_shape_position};
use crate::world::chunk::ChunkStore;

/// Native wire IDs remain independent of local request options.
#[derive(Clone, Copy, Debug)]
pub enum ServerParticleKind {
    EndRod,
}

impl ServerParticleKind {
    /// Registry order in the 26.2 reference, not azalea's wire enum.
    pub fn from_id(id: u32) -> Option<Self> {
        match id {
            27 => Some(Self::EndRod),
            _ => None,
        }
    }

    fn override_limiter(self) -> bool {
        match self {
            Self::EndRod => false,
        }
    }
}

impl ParticleStore {
    /// Vanilla `ClientLevel.addDestroyBlockEffect`: a grid over each shape box.
    pub fn add_destroy_block_effect(
        &mut self,
        pos: BlockPos,
        state: BlockState,
        registry: &BlockRegistry,
        chunks: &ChunkStore,
        biome_climate: &HashMap<u32, BiomeClimate>,
    ) {
        if is_air(state) {
            return;
        }
        let block_id = block_id(state);
        if matches!(block_id, "barrier" | "structure_void") {
            return;
        }

        // grass_block is exempt: its terrain particle texture is dirt.
        let mut color = [0.6f32; 3];
        let faces = registry.get_textures(state);
        if let Some(faces) = faces
            && faces.tint != Tint::None
            && block_id != "grass_block"
        {
            let tint = match faces.tint {
                Tint::Redstone => crate::world::block::redstone_wire_rgb(state),
                Tint::Stem => crate::world::block::stem_rgb(state),
                Tint::Constant(color) => crate::renderer::chunk::mesher::int_to_rgb(color as i32),
                _ => self.blend_tint(faces.tint, state, pos, chunks, biome_climate),
            };
            for (c, t) in color.iter_mut().zip(tint) {
                *c *= t;
            }
        }

        let region = match faces {
            Some(faces) => self
                .uv_map
                .get_region(faces.particle.as_deref().unwrap_or(&faces.top)),
            None => self.uv_map.get_region(""),
        };
        let light = world_brightness(chunks, pos.x, pos.y, pos.z);
        let boxes: &[LocalBox] = block_shape::outline_shape(state);
        let shape_origin = outline_shape_position(state, pos.x, pos.y, pos.z);
        let generator = match terrain::get_generator(self) {
            Ok(generator) => generator,
            Err(error) => {
                tracing::error!(?error, "Terrain generator is not initialized");
                return;
            }
        };

        for b in boxes {
            let width_x = (b[3] - b[0]).min(1.0);
            let width_y = (b[4] - b[1]).min(1.0);
            let width_z = (b[5] - b[2]).min(1.0);
            let count_x = ((width_x / 0.25).ceil() as i32).max(2);
            let count_y = ((width_y / 0.25).ceil() as i32).max(2);
            let count_z = ((width_z / 0.25).ceil() as i32).max(2);
            for xx in 0..count_x {
                for yy in 0..count_y {
                    for zz in 0..count_z {
                        let rel_x = (xx as f64 + 0.5) / count_x as f64;
                        let rel_y = (yy as f64 + 0.5) / count_y as f64;
                        let rel_z = (zz as f64 + 0.5) / count_z as f64;
                        let position = DVec3::new(
                            shape_origin.x + rel_x * width_x + b[0],
                            shape_origin.y + rel_y * width_y + b[1],
                            shape_origin.z + rel_z * width_z + b[2],
                        );
                        if let Err(error) = generator.create(
                            &mut self.source_spawns(),
                            TerrainOptions {
                                region,
                                color,
                                light,
                            },
                            position,
                            DVec3::new(rel_x - 0.5, rel_y - 0.5, rel_z - 0.5),
                        ) {
                            tracing::error!(?error, "Terrain creation failed");
                        }
                    }
                }
            }
        }
    }

    /// Vanilla `LivingEntity.spawnItemParticles`: crumbs along the look
    /// direction.
    pub fn add_item_use_particles(
        &mut self,
        count: u32,
        texture: &str,
        eye_pos: DVec3,
        x_rot_deg: f32,
        y_rot_deg: f32,
        chunks: &ChunkStore,
    ) {
        if !self.uv_map.has_region(texture) {
            return;
        }
        let region = self.uv_map.get_region(texture);
        let x_rot = -(x_rot_deg as f64).to_radians();
        let y_rot = -(y_rot_deg as f64).to_radians();
        let light = world_brightness(
            chunks,
            eye_pos.x.floor() as i32,
            eye_pos.y.floor() as i32,
            eye_pos.z.floor() as i32,
        );
        let generator = match terrain::get_item_generator(self) {
            Ok(generator) => generator,
            Err(error) => {
                tracing::error!(?error, "Item debris generator is not initialized");
                return;
            }
        };
        for _ in 0..count {
            let d = dvec3(
                (fastrand::f64() - 0.5) * 0.1,
                fastrand::f64() * 0.1 + 0.1,
                0.0,
            );
            let d = rot_y(rot_x(d, x_rot), y_rot);
            let p = dvec3(
                (fastrand::f64() - 0.5) * 0.3,
                -fastrand::f64() * 0.6 - 0.3,
                0.6,
            );
            let p = rot_y(rot_x(p, x_rot), y_rot) + eye_pos;
            if let Err(error) = generator.create(
                &mut self.source_spawns(),
                (region, light),
                p,
                dvec3(d.x, d.y + 0.05, d.z),
            ) {
                tracing::error!(?error, "Item debris creation failed");
            }
        }
    }

    /// Count 0 is directional; otherwise scatter each requested position first.
    #[allow(clippy::too_many_arguments)]
    pub fn add_particles_from_packet(
        &mut self,
        kind: ServerParticleKind,
        override_limiter: bool,
        pos: DVec3,
        dist: DVec3,
        max_speed: f64,
        count: u32,
        camera_pos: DVec3,
    ) {
        if count == 0 {
            self.add_server_particle(kind, override_limiter, pos, dist * max_speed, camera_pos);
            return;
        }
        for _ in 0..count {
            let scatter = dvec3(
                next_gaussian() * dist.x,
                next_gaussian() * dist.y,
                next_gaussian() * dist.z,
            );
            let vel = dvec3(next_gaussian(), next_gaussian(), next_gaussian()) * max_speed;
            self.add_server_particle(kind, override_limiter, pos + scatter, vel, camera_pos);
        }
    }

    fn add_server_particle(
        &mut self,
        kind: ServerParticleKind,
        override_limiter: bool,
        pos: DVec3,
        vel: DVec3,
        camera_pos: DVec3,
    ) {
        let generator = match kind {
            ServerParticleKind::EndRod => end_rod::get_generator(self),
        };
        let generator = match generator {
            Ok(generator) => generator,
            Err(error) => {
                tracing::error!(?error, "Server particle generator is not initialized");
                return;
            }
        };
        let mut context = if override_limiter || kind.override_limiter() {
            self.source_spawns()
        } else {
            self.local_spawns(camera_pos)
        };
        if let Err(error) = generator.create(&mut context, (), pos, vel) {
            tracing::error!(?error, "Server particle creation failed");
        }
    }

    /// The block's biome tint using vanilla's default blend radius.
    fn blend_tint(
        &self,
        tint: Tint,
        state: BlockState,
        pos: BlockPos,
        chunks: &ChunkStore,
        biome_climate: &HashMap<u32, BiomeClimate>,
    ) -> [f32; 3] {
        let sample_y = tint_sample_y(tint, state, pos.y);
        blend_color(pos.x, pos.z, |x, z| {
            let climate = biome_climate
                .get(&chunks.biome_id(x, sample_y, z))
                .copied()
                .unwrap_or_default();
            match tint {
                Tint::Grass | Tint::DoubleGrass => {
                    grass_color(&climate, &self.grass_colormap, x, z)
                }
                Tint::Foliage => foliage_color(&climate, &self.foliage_colormap),
                Tint::DryFoliage => dry_foliage_color(&climate, &self.dry_foliage_colormap),
                Tint::Water => climate.water_color,
                // State/fixed colors are resolved by the caller.
                Tint::None | Tint::Constant(_) | Tint::Redstone | Tint::Stem => [1.0; 3],
            }
        })
    }
}

fn rot_x(v: DVec3, angle: f64) -> DVec3 {
    let (sin, cos) = angle.sin_cos();
    dvec3(v.x, v.y * cos - v.z * sin, v.y * sin + v.z * cos)
}

fn rot_y(v: DVec3, angle: f64) -> DVec3 {
    let (sin, cos) = angle.sin_cos();
    dvec3(v.x * cos + v.z * sin, v.y, -v.x * sin + v.z * cos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::tests::{close, frames, store_with_regions};
    use crate::world::block::find_state;
    use crate::world::block::registry::FaceTextures;

    #[test]
    fn destroy_cube_and_bottom_slab_emit_known_grids_texture_and_tint() {
        let chunks = ChunkStore::with_origin_chunk();
        let pos = BlockPos::new(2, 80, 3);
        for (block, properties, ys, tint, expected_color) in [
            (
                "stone",
                &[][..],
                &[0.125, 0.375, 0.625, 0.875][..],
                Tint::None,
                [153, 153, 153, 255],
            ),
            (
                "oak_slab",
                &[("type", "bottom")][..],
                &[0.125, 0.375][..],
                Tint::Foliage,
                [77, 38, 153, 255],
            ),
        ] {
            let mut faces = FaceTextures::uniform("block/top", tint);
            faces.particle = Some("block/debris".into());
            let registry = BlockRegistry::with_textures(HashMap::from([(block.to_owned(), faces)]));
            let mut store =
                store_with_regions([("block/top", frames()[0]), ("block/debris", frames()[2])]);
            store.foliage_colormap =
                std::sync::Arc::new(crate::renderer::chunk::mesher::Colormap::constant([
                    128, 64, 255,
                ]));
            fastrand::seed(63);
            store.add_destroy_block_effect(
                pos,
                find_state(block, properties),
                &registry,
                &chunks,
                &HashMap::new(),
            );
            let anchor = dvec3(2.0, 80.0, 3.0);
            assert!(store.extract(0.0, anchor, &chunks).is_empty());
            store.tick(&chunks);
            let output = store.extract(0.0, anchor, &chunks);
            let mut expected = Vec::new();
            for x in [0.125, 0.375, 0.625, 0.875] {
                for &y in ys {
                    for z in [0.125, 0.375, 0.625, 0.875] {
                        expected.push([x, y, z]);
                    }
                }
            }
            assert_eq!(output.len(), if block == "stone" { 64 } else { 32 });
            for (quad, position) in output.iter().zip(expected) {
                assert_eq!(quad.pos, position);
                assert_eq!(quad.color.to_le_bytes(), expected_color);
                assert!(!quad.translucent);
                assert!((0.25..=0.375).contains(&quad.u0));
                assert!((0.25..=0.375).contains(&quad.u1));
                close(quad.u0 - quad.u1, 0.03125);
                close(quad.v1 - quad.v0, 0.125);
            }
        }
    }

    #[test]
    fn item_use_offsets_rotate_about_eye_in_pitch_then_yaw_order() {
        let chunks = ChunkStore::with_origin_chunk();
        let eye = dvec3(4.0, 81.5, 6.0);
        for (pitch, yaw) in [(0.0, 0.0), (0.0, 90.0), (90.0, 0.0), (90.0, 90.0)] {
            let mut store = store_with_regions([("item/apple", frames()[2])]);
            // The first two draws are velocity; the next two set the offset.
            // Right-angle expected coordinates are independent of rot_x/rot_y.
            let mut random = fastrand::Rng::with_seed(73);
            random.f64();
            random.f64();
            let x = (random.f64() - 0.5) * 0.3;
            let y = -random.f64() * 0.6 - 0.3;
            let expected = match (pitch, yaw) {
                (0.0, 0.0) => [x, y, 0.6],
                (0.0, 90.0) => [-0.6, y, x],
                (90.0, 0.0) => [x, 0.6, -y],
                _ => [y, 0.6, x],
            };
            fastrand::seed(73);
            store.add_item_use_particles(1, "item/apple", eye, pitch, yaw, &chunks);
            store.tick(&chunks);
            let output = store.extract(0.0, eye, &chunks);
            assert_eq!(output.len(), 1);
            for (actual, expected) in output[0].pos.into_iter().zip(expected) {
                close(actual, expected as f32);
            }
        }
    }
}
