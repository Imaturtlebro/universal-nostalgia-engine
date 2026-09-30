use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::PrimitiveTopology;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use vbsp::Bsp;

const MAP_NAME: &str = "gm_construct.bsp";
const GMOD_ASSET_DIRS: [&str; 2] = [
    r"D:\GModAssets\garrysmod",
    r"D:\gmodassets\garrysmod",
];

/// Source view height above `info_player_start` origin.
const PLAYER_EYE_HEIGHT: f32 = 64.0;

#[derive(Resource)]
pub struct LoadedMap {
    pub chunks: Vec<Mesh>,
    pub spawn_position: Vec3,
    pub spawn_yaw: f32,
    pub spawn_pitch: f32,
}

pub fn gmod_asset_dirs() -> [&'static str; 2] {
    GMOD_ASSET_DIRS
}

pub fn find_gmod_dir() -> Option<PathBuf> {
    GMOD_ASSET_DIRS
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_dir())
}

/// Source is Z-up right-handed, Bevy is Y-up right-handed, so this is a pure
/// -90 degree rotation about X: (x, y, z) -> (x, z, -y). Winding is preserved.
fn source_to_bevy(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2], -v[1]]
}

/// Side length of a spatial chunk, in Source units. gm_construct spans roughly
/// 30k units, so this yields a few hundred chunks: coarse enough that draw
/// calls stay low, fine enough that turning around culls most of the map.
const CHUNK_SIZE: f32 = 1024.0;

type ChunkKey = [i32; 3];

#[derive(Default)]
struct ChunkBuffers {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
}

fn chunk_key(center: Vec3) -> ChunkKey {
    [
        (center.x / CHUNK_SIZE).floor() as i32,
        (center.y / CHUNK_SIZE).floor() as i32,
        (center.z / CHUNK_SIZE).floor() as i32,
    ]
}

fn triangle_center(corners: [[f32; 3]; 3]) -> Vec3 {
    (Vec3::from(corners[0]) + Vec3::from(corners[1]) + Vec3::from(corners[2])) / 3.0
}

fn geometric_normal(corners: [[f32; 3]; 3]) -> Vec3 {
    let a = Vec3::from(corners[0]);
    let b = Vec3::from(corners[1]);
    let c = Vec3::from(corners[2]);
    (b - a).cross(c - a)
}

fn parse_vec3(raw: &str) -> Option<[f32; 3]> {
    let mut values = [0.0_f32; 3];
    for (slot, part) in values.iter_mut().zip(raw.split_whitespace()) {
        *slot = part.parse().ok()?;
    }
    Some(values)
}

pub fn load_map() -> Option<LoadedMap> {
    let map_path = find_gmod_dir()?.join("maps").join(MAP_NAME);
    println!("[bsp] loading {:?}", map_path);

    let started = Instant::now();
    let data = std::fs::read(&map_path)
        .map_err(|err| {
            println!("[bsp] FAILED to read file: {:?}", err);
        })
        .ok()?;
    println!(
        "[bsp] read {} MB in {:?}",
        data.len() / (1024 * 1024),
        started.elapsed()
    );

    let parse_started = Instant::now();
    let bsp = match Bsp::read(&data) {
        Ok(bsp) => bsp,
        Err(err) => {
            println!("[bsp] FAILED to parse: {:?}", err);
            return None;
        }
    };
    println!(
        "[bsp] parsed in {:?}: {} faces, {} vertices, {} texture infos",
        parse_started.elapsed(),
        bsp.faces.len(),
        bsp.vertices.len(),
        bsp.textures_info.len()
    );

    let world = bsp.models().next()?;
    println!(
        "[bsp] world model bounds: {:?} .. {:?}",
        world.mins, world.maxs
    );

    // Bucket every triangle into a spatial grid cell so each cell becomes its
    // own mesh entity. Bevy derives a Mesh's Aabb from its vertices, so
    // separate entities let the renderer frustum-cull chunks it cannot see.
    let mut chunks: HashMap<ChunkKey, ChunkBuffers> = HashMap::new();
    let mut visible_faces = 0_u32;
    let mut triangles = 0_u32;
    let mut flipped_triangles = 0_u32;

    for face in world.faces() {
        if !face.is_visible() {
            continue;
        }
        visible_faces += 1;

        let plane_normal = face.normal();
        let normal = source_to_bevy([plane_normal.x, plane_normal.y, plane_normal.z]);

        for triangle in face.triangulate() {
            let mut corners = [
                source_to_bevy([triangle[0].x, triangle[0].y, triangle[0].z]),
                source_to_bevy([triangle[1].x, triangle[1].y, triangle[1].z]),
                source_to_bevy([triangle[2].x, triangle[2].y, triangle[2].z]),
            ];

            // The BSP does not guarantee a consistent winding order, so derive
            // it: the geometric normal must agree with the face plane normal
            // (which points outwards), otherwise swap two corners.
            if geometric_normal(corners).dot(Vec3::from(normal)) < 0.0 {
                corners.swap(1, 2);
                flipped_triangles += 1;
            }

            let key = chunk_key(triangle_center(corners));
            let chunk = chunks.entry(key).or_default();

            for corner in corners {
                chunk.positions.push(corner);
                chunk.normals.push(normal);
            }
            triangles += 1;
        }
    }

    let meshes: Vec<Mesh> = chunks
        .into_values()
        .map(|chunk| {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, chunk.positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, chunk.normals);
            mesh
        })
        .collect();

    println!(
        "[bsp] built {visible_faces} visible faces into {triangles} triangles \
         across {} spatial chunks ({flipped_triangles} had reversed winding)",
        meshes.len()
    );

    let (spawn_position, spawn_yaw, spawn_pitch) = find_spawn_point(&bsp);

    println!("[bsp] first 10 textures in use:");
    for texture in bsp.textures().take(10) {
        println!("[bsp]   {}", texture.name());
    }

    Some(LoadedMap {
        chunks: meshes,
        spawn_position,
        spawn_yaw,
        spawn_pitch,
    })
}

fn find_spawn_point(bsp: &Bsp) -> (Vec3, f32, f32) {
    let mut start_count = 0_u32;
    let mut chosen: Option<(Vec3, f32, f32)> = None;

    for entity in bsp.entities.iter() {
        if entity.prop("classname") != Some("info_player_start") {
            continue;
        }
        start_count += 1;

        if chosen.is_some() {
            continue;
        }

        let Some(origin) = entity.prop("origin").and_then(parse_vec3) else {
            continue;
        };

        // Source entity angles are (pitch, yaw, roll), all in degrees.
        let (pitch, yaw) = entity
            .prop("angles")
            .and_then(parse_vec3)
            .map(|[pitch, yaw, _]| (pitch, yaw))
            .unwrap_or((0.0, 0.0));

        let mut position = Vec3::from(source_to_bevy(origin));
        position.y += PLAYER_EYE_HEIGHT;

        chosen = Some((
            position,
            yaw.to_radians() - std::f32::consts::FRAC_PI_2,
            -pitch.to_radians(),
        ));
    }

    println!("[bsp] found {start_count} info_player_start entities");

    match chosen {
        Some(spawn) => {
            println!(
                "[bsp] spawning at {:?} (yaw {:.1} deg, pitch {:.1} deg)",
                spawn.0,
                spawn.1.to_degrees(),
                spawn.2.to_degrees()
            );
            spawn
        }
        None => {
            println!("[bsp] no usable spawn point, defaulting to map centre");
            (
                Vec3::new(0.0, PLAYER_EYE_HEIGHT, 0.0),
                0.0,
                0.0,
            )
        }
    }
}
