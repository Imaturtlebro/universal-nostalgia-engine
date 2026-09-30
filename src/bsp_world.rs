use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::PrimitiveTopology;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use vbsp::{Bsp, Vector};

const MAP_NAME: &str = "gm_construct.bsp";
const GMOD_ASSET_DIRS: [&str; 2] = [
    r"D:\GModAssets\garrysmod",
    r"D:\gmodassets\garrysmod",
];

/// Source view height above `info_player_start` origin.
const PLAYER_EYE_HEIGHT: f32 = 64.0;

/// One draw-call group: a chunk mesh plus the index of the material bucket
/// whose texture name it was grouped by.
pub type ChunkMesh = (usize, Mesh);

#[derive(Resource)]
pub struct LoadedMap {
    pub chunks: Vec<ChunkMesh>,
    /// Lowercased texture name per material bucket index.
    pub bucket_names: Vec<String>,
    pub triangles_total: usize,
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

/// Side length of a spatial chunk, in Source units.
///
/// gm_construct spans roughly 30k units. At 1024 units the map split into
/// 1211 chunk meshes, and the per-frame draw call overhead on an Intel UHD 600
/// cost more than the frustum culling saved. Coarser cells trade a little
/// culling precision for far fewer draw calls.
const CHUNK_SIZE: f32 = 6144.0;

type ChunkKey = [i32; 3];

#[derive(Default)]
struct ChunkBuffers {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
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

    // Bucket every triangle into a spatial grid cell, keyed by material bucket,
    // so each cell becomes its own mesh entity. Bevy derives a Mesh's Aabb from
    // its vertices, so separate entities let the renderer frustum-cull chunks
    // it cannot see. Keying by material too keeps a single draw call per
    // (chunk, material) pair instead of one per triangle.
    let mut chunks: HashMap<ChunkKey, HashMap<usize, ChunkBuffers>> = HashMap::new();
    let mut visible_faces = 0_u32;
    let mut triangles = 0_u32;
    let mut flipped_triangles = 0_u32;
    let mut buckets: HashMap<String, usize> = HashMap::new();
    let mut bucket_names: Vec<String> = Vec::new();

    for face in world.faces() {
        if !face.is_visible() {
            continue;
        }
        visible_faces += 1;

        let plane_normal = face.normal();
        let normal = source_to_bevy([plane_normal.x, plane_normal.y, plane_normal.z]);

        // texinfo holds the projection axes used to derive UVs; the texture
        // name it points at decides which material bucket this face lands in.
        let texture = face.texture();
        let texture_name = texture.name().to_ascii_lowercase();
        let bucket = *buckets.entry(texture_name.clone()).or_insert_with(|| {
            bucket_names.push(texture_name.clone());
            bucket_names.len() - 1
        });

        for triangle in face.triangulate() {
            // UVs are computed from the *Source* position: the texinfo
            // projection matrices are defined in Source's coordinate space.
            let mut source_corners = [
                [triangle[0].x, triangle[0].y, triangle[0].z],
                [triangle[1].x, triangle[1].y, triangle[1].z],
                [triangle[2].x, triangle[2].y, triangle[2].z],
            ];
            let mut uvs = [
                texture.uv(Vector::from(source_corners[0])),
                texture.uv(Vector::from(source_corners[1])),
                texture.uv(Vector::from(source_corners[2])),
            ];

            let mut corners = [
                source_to_bevy(source_corners[0]),
                source_to_bevy(source_corners[1]),
                source_to_bevy(source_corners[2]),
            ];

            // The BSP does not guarantee a consistent winding order, so derive
            // it: the geometric normal must agree with the face plane normal
            // (which points outwards), otherwise swap two corners.
            if geometric_normal(corners).dot(Vec3::from(normal)) < 0.0 {
                corners.swap(1, 2);
                source_corners.swap(1, 2);
                uvs.swap(1, 2);
                flipped_triangles += 1;
            }

            let key = chunk_key(triangle_center(corners));
            let chunk = chunks
                .entry(key)
                .or_default()
                .entry(bucket)
                .or_default();

            for (index, corner) in corners.into_iter().enumerate() {
                chunk.positions.push(corner);
                chunk.normals.push(normal);
                chunk.uvs.push(uvs[index]);
            }
            triangles += 1;
        }
    }

    // Merge each cell's per-bucket buffers back into a single mesh.
    //
    // Keeping the material split per cell would multiply draw calls by the
    // number of distinct textures touching that cell, which on a 187-texture
    // map is worse than the culling win. A single material per cell keeps one
    // draw call per visible cell; the cell's dominant texture is used for its
    // colour, so a cell still reads as roughly one surface type.
    let mut cell_buckets: HashMap<usize, usize> = HashMap::new();
    let mut chunk_count = 0_u32;
    let mut meshes: Vec<ChunkMesh> = Vec::new();

    for (cell, buckets_in_cell) in chunks {
        let mut total_triangles = 0_usize;
        let mut dominant_bucket = 0_usize;
        let mut dominant_triangles = 0_usize;

        for (bucket, buffers) in &buckets_in_cell {
            let cell_triangles = buffers.positions.len() / 3;
            total_triangles += cell_triangles;
            if cell_triangles > dominant_triangles {
                dominant_triangles = cell_triangles;
                dominant_bucket = *bucket;
            }
        }

        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total_triangles * 3);
        let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total_triangles * 3);
        let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(total_triangles * 3);

        for (_, buffers) in buckets_in_cell {
            positions.extend_from_slice(&buffers.positions);
            normals.extend_from_slice(&buffers.normals);
            uvs.extend_from_slice(&buffers.uvs);
        }

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);

        cell_buckets.insert(cell, dominant_bucket);
        meshes.push((dominant_bucket, mesh));
        chunk_count += 1;
    }

    println!(
        "[bsp] built {visible_faces} visible faces into {triangles} triangles \
         across {chunk_count} spatial chunks at {CHUNK_SIZE} units \
         ({flipped_triangles} had reversed winding)"
    );
    println!(
        "[bsp] {chunk_count} distinct textures used, drawn from {} material buckets",
        cell_buckets.len()
    );
    // Only the first few buckets are listed: there are hundreds, and dumping
    // them all buries the rest of the startup log.
    for (index, name) in bucket_names.iter().take(10).enumerate() {
        println!("[bsp]   material {index}: {name}");
    }
    if bucket_names.len() > 10 {
        println!(
            "[bsp]   ... and {} more material buckets",
            bucket_names.len() - 10
        );
    }

    let (spawn_position, spawn_yaw, spawn_pitch) = find_spawn_point(&bsp);

    println!("[bsp] first 10 textures in use:");
    for texture in bsp.textures().take(10) {
        println!("[bsp]   {}", texture.name());
    }

    Some(LoadedMap {
        chunks: meshes,
        bucket_names,
        triangles_total: triangles as usize,
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
