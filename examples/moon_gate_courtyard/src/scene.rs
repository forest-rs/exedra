// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{Result, geometry};
use exedra_assembly::{Assembly, InstanceId, PartId};
use exedra_constructive::ir::{Placement3, Recipe};
use exedra_mesh::Mesh;

#[derive(Default)]
pub(crate) struct Scene {
    pub(crate) assembly: Assembly,
    serial: u32,
    pub(crate) parent: Option<InstanceId>,
}
impl Scene {
    pub(crate) fn group<T>(
        &mut self,
        key: &str,
        placement: Placement3,
        build: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let frame = self.assembly.add_frame(self.parent, key, placement)?;
        let previous = self.parent.replace(frame);
        let result = build(self);
        self.parent = previous;
        result
    }

    pub(crate) fn recipe(&mut self, key: &str, recipe: Recipe, material: &str) -> Result<PartId> {
        let id = self.assembly.add_recipe_part(key, recipe)?;
        self.assembly.set_default_slot(id, "surface")?;
        self.assembly.set_part_material(id, "surface", material)?;
        Ok(id)
    }
    pub(crate) fn mesh(&mut self, key: &str, mesh: Mesh, material: &str) -> Result<PartId> {
        let id = self.assembly.add_baked_part(key, mesh, &["surface"])?;
        self.assembly.set_default_slot(id, "surface")?;
        self.assembly.set_part_material(id, "surface", material)?;
        Ok(id)
    }
    pub(crate) fn place(
        &mut self,
        part: PartId,
        placement: Placement3,
        material: Option<&str>,
    ) -> Result<InstanceId> {
        let key = format!(
            "{}-{}",
            self.assembly.part(part).ok_or("missing part")?.key(),
            self.serial
        );
        self.serial += 1;
        let id = self
            .assembly
            .add_instance(self.parent, &key, part, placement)?;
        if let Some(material) = material {
            self.assembly.bind_material(id, "surface", material)?;
        }
        Ok(id)
    }
    pub(crate) fn at(&mut self, part: PartId, p: [f64; 3]) -> Result<InstanceId> {
        self.place(part, Placement3::translate(p[0], p[1], p[2]), None)
    }
    pub(crate) fn block(
        &mut self,
        key: &str,
        size: [f64; 3],
        p: [f64; 3],
        material: &str,
        bevel: f64,
    ) -> Result<PartId> {
        let part = self.recipe(key, geometry::block(size, bevel)?, material)?;
        self.at(part, p)?;
        Ok(part)
    }
}
