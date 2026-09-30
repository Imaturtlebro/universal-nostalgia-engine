use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::PrimitiveTopology;
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
    pub mesh: Mesh,
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

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut visible_faces = 0_u32;
    let mut triangles = 0_u32;

    for face in world.faces() {
        if !face.is_visible() {
            continue;
        }
        visible_faces += 1;

        let normal = source_to_bevy([face.normal().x, face.normal().y, face.normal().z]);

        for triangle in face.triangulate() {
            for corner in triangle {
                positions.push(source_to_bevy([corner.x, corner.y, corner.z]));
                normals.push(normal);
            }
            triangles += 1;
        }
    }
    println!(
        "[bsp] built {visible_faces} visible faces into {triangles} triangles ({} vertices)",
        positions.len()
    );

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);

    let (spawn_position, spawn_yaw, spawn_pitch) = find_spawn_point(&bsp);

    println!("[bsp] first 10 textures in use:");
    for texture in bsp.textures().take(10) {
        println!("[bsp]   {}", texture.name());
    }

    Some(LoadedMap {
        mesh,
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
