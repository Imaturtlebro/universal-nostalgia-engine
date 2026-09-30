//! Brush collision extracted from the Source BSP.
//!
//! A Source brush is a convex volume defined by a set of half-space planes.
//! The player is resolved against those as a vertical capsule: for each plane
//! the closest approach of the capsule's spine is measured, and the deepest
//! penetrating plane pushes back.

use bevy::math::Vec3;
use std::collections::HashMap;
use vbsp::{Bsp, Vector};

/// Half-width of the player capsule, in Source units.
pub const PLAYER_RADIUS: f32 = 24.0;

/// Standing capsule height, in Source units. GMod players are 72 units tall.
pub const PLAYER_HEIGHT: f32 = 72.0;
pub const CROUCH_HEIGHT: f32 = 44.0;

/// Eye offset from the feet when standing / crouched.
pub const EYE_HEIGHT: f32 = 64.0;
pub const CROUCH_EYE_HEIGHT: f32 = 34.0;

/// Largest ledge the player walks up without jumping.
pub const STEP_HEIGHT: f32 = 20.0;

/// Broadphase grid cell size, in Source units.
const GRID_CELL: f32 = 256.0;

/// One half-space in Bevy coordinates.
#[derive(Clone, Copy)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
}

impl Plane {
    /// Signed distance from `point` to the plane. Negative means behind it,
    /// which for a solid brush means inside.
    #[inline]
    pub fn distance(&self, point: Vec3) -> f32 {
        self.normal.dot(point) - self.dist
    }
}

/// A convex brush, stored as planes plus a broadphase AABB.
pub struct Brush {
    pub planes: Vec<Plane>,
    pub min: Vec3,
    pub max: Vec3,
}

/// Uniform-grid broadphase over the brushes.
pub struct CollisionWorld {
    brushes: Vec<Brush>,
    /// Grid cell -> indices of brushes whose AABB overlaps that cell.
    cells: HashMap<[i32; 3], Vec<u32>>,
}

fn cell_of(point: Vec3) -> [i32; 3] {
    [
        (point.x / GRID_CELL).floor() as i32,
        (point.y / GRID_CELL).floor() as i32,
        (point.z / GRID_CELL).floor() as i32,
    ]
}

impl CollisionWorld {
    pub fn brush_count(&self) -> usize {
        self.brushes.len()
    }

    /// How many brushes the broadphase reports near a point. The debug overlay
    /// shows this so it is visible that the grid is doing work rather than
    /// testing every brush in the map each frame.
    pub fn brushes_near(&self, point: Vec3, height: f32) -> usize {
        self.candidates(point, height).len()
    }

    /// Brush indices whose AABB may overlap the capsule's vertical extent,
    /// deduplicated across the cells the capsule spans.
    fn candidates(&self, point: Vec3, height: f32) -> Vec<u32> {
        let min_cell = cell_of(point - Vec3::Y * (height * 0.5));
        let max_cell = cell_of(point + Vec3::Y * (height * 0.5));

        let mut found: Vec<u32> = Vec::new();
        for x in min_cell[0]..=max_cell[0] {
            for y in min_cell[1]..=max_cell[1] {
                for z in min_cell[2]..=max_cell[2] {
                    if let Some(indices) = self.cells.get(&[x, y, z]) {
                        for index in indices {
                            if !found.contains(index) {
                                found.push(*index);
                            }
                        }
                    }
                }
            }
        }
        found
    }

    /// Pushes a capsule out of the brushes around it.
    ///
    /// `feet` and `head` are the capsule's extremes; the spine runs between
    /// them inset by the radius. Resolution repeats because escaping one brush
    /// can push into another in tight corners.
    pub fn resolve(&self, feet: Vec3, head: Vec3, radius: f32) -> (Vec3, Option<Vec3>) {
        let mut position = (feet + head) * 0.5;
        let half_height = ((head.y - feet.y) * 0.5 - radius).max(0.0);
        let mut contact: Option<Vec3> = None;

        for _ in 0..4 {
            let mut deepest = 0.0_f32;
            let mut deepest_normal = Vec3::ZERO;

            for index in self.candidates(position, head.y - feet.y) {
                let brush = &self.brushes[index as usize];

                // Cheap AABB reject before touching the planes.
                let reach = radius + half_height;
                if position.x + reach < brush.min.x
                    || position.x - reach > brush.max.x
                    || position.z + reach < brush.min.z
                    || position.z - reach > brush.max.z
                {
                    continue;
                }

                let spine_bottom = position - Vec3::Y * half_height;
                let spine_top = position + Vec3::Y * half_height;

                for plane in &brush.planes {
                    let closest = plane
                        .distance(spine_bottom)
                        .min(plane.distance(spine_top));
                    if closest >= radius {
                        continue;
                    }

                    let penetration = radius - closest;
                    if penetration > deepest {
                        deepest = penetration;
                        deepest_normal = plane.normal;
                    }
                }
            }

            if deepest <= f32::EPSILON {
                break;
            }

            position += deepest_normal * deepest;
            contact = Some(deepest_normal);
        }

        (position, contact)
    }
}

/// Converts a Source plane to Bevy coordinates.
///
/// `source_to_bevy` is a pure rotation, and rotations preserve dot products:
/// `dot(Rn, Rp) == dot(n, p)`. The plane's `dist` therefore carries over
/// unchanged; only the normal needs rotating.
fn convert_plane(normal: Vector, dist: f32) -> Plane {
    Plane {
        normal: Vec3::new(normal.x, normal.z, -normal.y).normalize(),
        dist,
    }
}

/// Tight AABB of the convex volume bounded by `planes`.
///
/// The volume satisfies `dot(n_i, p) <= d_i` for every plane. Substituting a
/// point on the axis `a` as `p = t * e_a` gives `t * n_i[a] <= d_i`, so the
/// upper bound along that axis is the smallest `d_i / n_i[a]` over planes
/// facing the same way, and the lower bound is the largest such ratio over
/// planes facing the other way.
fn brush_aabb(planes: &[Plane]) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let mut lower = [f32::MIN; 3];
    let mut upper = [f32::MAX; 3];

    for plane in planes {
        for axis in 0..3 {
            let component = plane.normal[axis];
            if component > 1.0e-6 {
                upper[axis] = upper[axis].min(plane.dist / component);
            } else if component < -1.0e-6 {
                lower[axis] = lower[axis].max(plane.dist / component);
            }
        }
    }

    for axis in 0..3 {
        if lower[axis] > upper[axis] {
            return None;
        }
        min[axis] = lower[axis];
        max[axis] = upper[axis];
    }

    Some((min, max))
}

/// Builds the collision world from every brush a player can collide with.
pub fn build_collision_world(bsp: &Bsp) -> CollisionWorld {
    let mut brushes: Vec<Brush> = Vec::new();
    let mut cells: HashMap<[i32; 3], Vec<u32>> = HashMap::new();

    for brush in &bsp.brushes {
        // SOLID, GRATE and WINDOW brushes are collidable. Water, mist and
        // trigger volumes are not, and neither are areaportals.
        if !brush.is_visible() {
            continue;
        }

        let first = brush.brush_side as usize;
        let count = brush.num_brush_sides as usize;
        if count == 0 {
            continue;
        }

        let mut planes: Vec<Plane> = Vec::with_capacity(count);
        for side_index in first..first + count {
            let Some(side) = bsp.brush_sides.get(side_index) else {
                continue;
            };
            // Bevelled sides are corner cuts, not flat half-spaces.
            if side.bevel != 0 {
                continue;
            }
            let Some(plane) = bsp.planes.get(side.plane as usize) else {
                continue;
            };
            planes.push(convert_plane(plane.normal, plane.dist));
        }

        if planes.len() < 4 {
            continue;
        }

        let Some((min, max)) = brush_aabb(&planes) else {
            continue;
        };

        let index = brushes.len() as u32;
        brushes.push(Brush { planes, min, max });

        let min_cell = cell_of(min);
        let max_cell = cell_of(max);
        for x in min_cell[0]..=max_cell[0] {
            for y in min_cell[1]..=max_cell[1] {
                for z in min_cell[2]..=max_cell[2] {
                    cells.entry([x, y, z]).or_default().push(index);
                }
            }
        }
    }

    CollisionWorld { brushes, cells }
}
